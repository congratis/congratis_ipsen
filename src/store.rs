use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use futures::TryStreamExt;
use reql::{cmd::connect::Options, r, Session};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, time::Duration};
use tokio::sync::{broadcast, watch};

const TABLE: &str = "ornaments";
const TOKEN_INDEX: &str = "token_hash";
const SESSIONS_TABLE: &str = "auth_sessions";
const CAMPAIGNS_TABLE: &str = "campaigns";
pub const LEGACY_CAMPAIGN_ID: &str = "legacy-default";
const INDEX_PROBE_TOKEN: &str = "__congratis_index_probe__";

fn token_index_options() -> reql::cmd::get_all::Options {
    let mut options = reql::cmd::get_all::Options::default();
    options.index = Some(TOKEN_INDEX.into());
    options
}

#[derive(Clone)]
struct ConnectionConfig {
    host: String,
    port: u16,
    user: String,
    password: String,
}

impl ConnectionConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            host: env::var("RETHINKDB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: env::var("RETHINKDB_PORT")
                .unwrap_or_else(|_| "28015".into())
                .parse()
                .context("RETHINKDB_PORT must be a valid TCP port")?,
            user: env::var("RETHINKDB_USER").unwrap_or_else(|_| "admin".into()),
            password: env::var("RETHINKDB_PASSWORD").unwrap_or_default(),
        })
    }

    async fn connect(&self) -> Result<Session> {
        let options = Options::new()
            .host(self.host.clone())
            .port(self.port)
            .user(self.user.clone())
            .password(self.password.clone());
        Ok(tokio::time::timeout(Duration::from_secs(10), r.connect(options))
            .await
            .context("timed out connecting to RethinkDB")??)
    }
}

#[derive(Clone)]
pub struct Store {
    session: Session,
    database: String,
    connection: ConnectionConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct GreetingEvent {
    pub ornament_number: i16,
    pub display_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ornament {
    pub id: i16,
    pub ornament_number: i16,
    #[serde(default)]
    pub campaign_id: Option<String>,
    pub token_hash: String,
    pub used: bool,
    #[serde(default)]
    pub activated: bool,
    #[serde(default)]
    pub revoked: bool,
    pub display_name: Option<String>,
    pub message: Option<String>,
    pub created_at: i64,
    pub used_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct WriteResult {
    #[serde(default)]
    inserted: u64,
    #[serde(default)]
    replaced: u64,
    #[serde(default)]
    errors: u64,
    first_error: Option<String>,
}

impl Store {
    pub async fn connect() -> Result<Self> {
        let connection = ConnectionConfig::from_env()?;
        let database = env::var("RETHINKDB_DB").unwrap_or_else(|_| "congratis".into());
        let session = connection.connect().await?;

        let store = Self {
            session,
            database,
            connection,
        };
        store.initialize().await?;
        Ok(store)
    }

    async fn first<T>(&self, query: reql::Command) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned + Unpin,
    {
        let mut stream = query.run::<_, T>(&self.session);
        Ok(stream.try_next().await?)
    }

    async fn initialize(&self) -> Result<()> {
        let databases = self
            .first::<Vec<String>>(r.db_list())
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no database list"))?;

        if !databases.iter().any(|name| name == &self.database) {
            let _: Option<Value> = self
                .first(r.db_create(self.database.as_str()))
                .await
                .context("creating RethinkDB database")?;
        }

        let tables = self
            .first::<Vec<String>>(r.db(self.database.as_str()).table_list())
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no table list"))?;

        if !tables.iter().any(|name| name == TABLE) {
            let _: Option<Value> = self
                .first(r.db(self.database.as_str()).table_create(TABLE))
                .await
                .context("creating ornaments table")?;
        }

        let indexes = self
            .first::<Vec<String>>(
                r.db(self.database.as_str())
                    .table(TABLE)
                    .index_list(),
            )
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no index list"))?;

        if !indexes.iter().any(|name| name == TOKEN_INDEX) {
            let _: Option<Value> = self
                .first(
                    r.db(self.database.as_str())
                        .table(TABLE)
                        .index_create(TOKEN_INDEX),
                )
                .await
                .context("creating token_hash index")?;
        }

        self.wait_for_token_index().await?;

        let tables = self.first::<Vec<String>>(
            r.db(self.database.as_str()).table_list()
        ).await?.ok_or_else(|| anyhow!("RethinkDB returned no table list"))?;
        if !tables.iter().any(|name| name == SESSIONS_TABLE) {
            let _: Option<Value> = self.first(
                r.db(self.database.as_str()).table_create(SESSIONS_TABLE)
            ).await.context("creating auth_sessions table")?;
        }

        let tables = self.first::<Vec<String>>(
            r.db(self.database.as_str()).table_list()
        ).await?.ok_or_else(|| anyhow!("RethinkDB returned no table list"))?;
        if !tables.iter().any(|name| name == CAMPAIGNS_TABLE) {
            let _: Option<Value> = self.first(
                r.db(self.database.as_str()).table_create(CAMPAIGNS_TABLE)
            ).await.context("creating campaigns table")?;
        }

        self.migrate_legacy_campaign().await?;
        Ok(())
    }

    /// Additively assigns all pre-multi-campaign records to one stable legacy campaign.
    /// It never rewrites existing campaign IDs and is safe to run on every startup.
    async fn migrate_legacy_campaign(&self) -> Result<()> {
        let ornaments = self.list_ornaments().await?;
        if ornaments.is_empty() || ornaments.iter().all(|item| item.campaign_id.is_some()) {
            return Ok(());
        }

        let existing_campaign: Option<Value> = self.first(
            r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(LEGACY_CAMPAIGN_ID)
        ).await?;
        if existing_campaign.is_none() {
            let now = Utc::now().timestamp();
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).insert(json!({
                    "id": LEGACY_CAMPAIGN_ID,
                    "name": "Существующая кампания",
                    "created_at": now,
                    "updated_at": now
                }))
            ).await?.ok_or_else(|| anyhow!("missing legacy campaign insert result"))?;
            if result.errors != 0 || result.inserted != 1 {
                // Another app instance may have completed the same idempotent insert.
                let found: Option<Value> = self.first(
                    r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(LEGACY_CAMPAIGN_ID)
                ).await?;
                if found.is_none() {
                    bail!("legacy campaign creation failed: {}", result.first_error.unwrap_or_default());
                }
            }
        }

        for ornament in ornaments.iter().filter(|item| item.campaign_id.is_none()) {
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(TABLE).get(ornament.id)
                    .update(json!({"campaign_id": LEGACY_CAMPAIGN_ID}))
            ).await?.ok_or_else(|| anyhow!("missing legacy ornament migration result"))?;
            if result.errors != 0 {
                bail!("legacy ornament migration failed for {}: {}", ornament.id, result.first_error.unwrap_or_default());
            }
        }

        let mut sessions_stream = r.db(self.database.as_str()).table(SESSIONS_TABLE)
            .run::<_, Value>(&self.session);
        let mut sessions = Vec::new();
        while let Some(session) = sessions_stream.try_next().await? {
            sessions.push(session);
        }
        drop(sessions_stream);

        for session in sessions {
            if session.get("role").and_then(Value::as_str) != Some("participant")
                || session.get("campaign_id").is_some()
            {
                continue;
            }
            let id = session.get("id").and_then(Value::as_str)
                .ok_or_else(|| anyhow!("participant session without an ID during campaign migration"))?
                .to_owned();
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(SESSIONS_TABLE).get(id.as_str())
                    .update(json!({"campaign_id": LEGACY_CAMPAIGN_ID}))
            ).await?.ok_or_else(|| anyhow!("missing participant session migration result"))?;
            if result.errors != 0 {
                bail!("participant session migration failed: {}", result.first_error.unwrap_or_default());
            }
        }

        let migrated = self.list_ornaments().await?;
        if migrated.iter().any(|item| item.campaign_id.is_none()) {
            bail!("legacy campaign migration verification failed: an invitation has no campaign assignment");
        }
        Ok(())
    }

    async fn wait_for_token_index(&self) -> Result<()> {
        let mut last_error = None;
        for _ in 0..40 {
            let query = r
                .db(self.database.as_str())
                .table(TABLE)
                .get_all(r.args((INDEX_PROBE_TOKEN, token_index_options())))
                .limit(1);
            let mut stream = query.run::<_, Ornament>(&self.session);
            match stream.try_next().await {
                Ok(_) => return Ok(()),
                Err(error) => {
                    last_error = Some(error.to_string());
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }

        bail!(
            "token_hash index did not become queryable: {}",
            last_error.unwrap_or_else(|| "unknown index readiness error".into())
        )
    }

    pub async fn list_ornaments(&self) -> Result<Vec<Ornament>> {
        // Read the base table stream and sort the bounded campaign collection
        // in Rust. This also avoids an ordered-stream query over an empty table,
        // which the pinned driver decodes inconsistently.
        let mut stream = r
            .db(self.database.as_str())
            .table(TABLE)
            .run::<_, Ornament>(&self.session);

        let mut ornaments = Vec::new();
        while let Some(ornament) = stream.try_next().await? {
            ornaments.push(ornament);
        }
        ornaments.sort_by_key(|ornament| ornament.ornament_number);
        Ok(ornaments)
    }

    pub async fn list_greetings(&self) -> Result<Vec<Ornament>> {
        Ok(self
            .list_ornaments()
            .await?
            .into_iter()
            .filter(|ornament| ornament.used)
            .collect())
    }

    pub async fn insert_invitations(&self, invitations: &[(i16, String)]) -> Result<()> {
        let existing = self.list_ornaments().await?;
        if !existing.is_empty() {
            bail!("ornaments table is not empty; refusing to overwrite invitations");
        }

        let mut numbers = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();
        for (number, token_hash) in invitations {
            if !(1..=35).contains(number) || !numbers.insert(*number) {
                bail!("invitation ornament numbers must be unique values between 1 and 35");
            }
            if !hashes.insert(token_hash) {
                bail!("duplicate invitation token hash in batch");
            }
        }

        let created_at = Utc::now().timestamp();
        let documents: Vec<Value> = invitations
            .iter()
            .map(|(number, token_hash)| {
                json!({
                    "id": number,
                    "ornament_number": number,
                    "campaign_id": LEGACY_CAMPAIGN_ID,
                    "token_hash": token_hash,
                    "used": false,
                    "activated": false,
                    "revoked": false,
                    "display_name": null,
                    "message": null,
                    "created_at": created_at,
                    "used_at": null
                })
            })
            .collect();

        let result = self
            .first::<WriteResult>(
                r.db(self.database.as_str())
                    .table(TABLE)
                    .insert(documents),
            )
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no insert result"))?;

        if result.errors != 0 || result.inserted != invitations.len() as u64 {
            bail!(
                "invitation insert incomplete: inserted={}, errors={}, first_error={:?}",
                result.inserted,
                result.errors,
                result.first_error
            );
        }
        Ok(())
    }

    pub async fn claim_invitation(
        &self,
        token_hash: &str,
        display_name: &str,
        message: &str,
    ) -> Result<Option<i16>> {
        // get_all(token_hash, index).limit(1) selects at most one document.
        // The conditional update function and its state change execute in one
        // atomic single-document write; no read-then-write race is possible.
        let update = reql::func!(|document| {
            r.branch(r.args((
                document.clone().get_field("used").eq(false),
                document.clone().merge(r.expr(json!({
                    "used": true,
                    "display_name": display_name,
                    "message": message,
                    "used_at": Utc::now().timestamp()
                }))),
                document
            )))
        });

        let query = r
            .db(self.database.as_str())
            .table(TABLE)
            .get_all(r.args((
                token_hash,
                token_index_options(),
            )))
            .limit(1)
            .update(update);

        let result = self
            .first::<WriteResult>(query)
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no invitation claim result"))?;

        if result.errors != 0 {
            bail!(
                "RethinkDB invitation claim failed: {}",
                result.first_error.unwrap_or_else(|| "unknown write error".into())
            );
        }

        if result.replaced != 1 {
            return Ok(None);
        }

        // The claimed document is read back by its indexed token. This read is
        // only for the response ornament number; it is not part of the claim.
        let mut stream = r
            .db(self.database.as_str())
            .table(TABLE)
            .get_all(r.args((
                token_hash,
                token_index_options(),
            )))
            .limit(1)
            .run::<_, Ornament>(&self.session);
        let claimed = stream
            .try_next()
            .await?
            .ok_or_else(|| anyhow!("claimed invitation disappeared after update"))?;
        Ok(Some(claimed.ornament_number))
    }

    pub async fn supervise_changefeed(self, sender: broadcast::Sender<GreetingEvent>) {
        let mut backoff_seconds = 1u64;
        loop {
            let outcome = match self.connection.connect().await {
                Ok(session) => self.consume_changefeed(&session, &sender, None).await,
                Err(error) => Err(error),
            };

            match outcome {
                Ok(()) => tracing::warn!("RethinkDB Changefeed ended; reconnecting"),
                Err(error) => tracing::error!(?error, "RethinkDB Changefeed failed; reconnecting"),
            }

            tokio::time::sleep(Duration::from_secs(backoff_seconds)).await;
            backoff_seconds = (backoff_seconds * 2).min(30);
        }
    }

    pub async fn subscribe_changefeed(
        &self,
        sender: &broadcast::Sender<GreetingEvent>,
        ready: watch::Sender<bool>,
    ) -> Result<()> {
        let session = self.connection.connect().await?;
        self.consume_changefeed(&session, sender, Some(&ready)).await
    }

    async fn consume_changefeed(
        &self,
        session: &Session,
        sender: &broadcast::Sender<GreetingEvent>,
        ready: Option<&watch::Sender<bool>>,
    ) -> Result<()> {
        let mut options = reql::cmd::changes::Options::default();
        options.include_initial = Some(true);
        options.include_states = Some(true);
        options.include_types = Some(true);

        // This session is dedicated to the long-lived feed. The reql driver
        // locks a Session while a Changefeed is active, so CRUD uses self.session.
        let mut stream = r
            .db(self.database.as_str())
            .table(TABLE)
            .changes(options)
            .run::<_, Value>(session);

        while let Some(change) = stream.try_next().await? {
            if change.get("state").and_then(Value::as_str) == Some("ready") {
                if let Some(ready) = ready {
                    let _ = ready.send(true);
                }
                continue;
            }

            if let Some(message) = change.get("error").and_then(Value::as_str) {
                bail!("RethinkDB Changefeed overflow/error: {message}");
            }

            let Some(new_value) = change.get("new_val") else {
                // State notifications and uninitial notifications are not greetings.
                continue;
            };
            if new_value.is_null() {
                continue;
            }

            let ornament: Ornament = serde_json::from_value(new_value.clone())?;
            if !ornament.used {
                continue;
            }

            let (Some(display_name), Some(message)) =
                (ornament.display_name, ornament.message)
            else {
                continue;
            };

            let _ = sender.send(GreetingEvent {
                ornament_number: ornament.ornament_number,
                display_name,
                message,
            });
        }

        Ok(())
    }

    pub async fn ping(&self) -> Result<()> {
        let databases = self
            .first::<Vec<String>>(r.db_list())
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no database list"))?;
        if !databases.iter().any(|name| name == &self.database) {
            bail!("configured RethinkDB database is missing");
        }
        Ok(())
    }
}

impl Store {
    pub async fn reissue_invitation(&self, ornament_number: i16, token_hash: &str) -> Result<bool> {
        let update = reql::func!(|document| {
            r.branch(
                r.args((
                    document.clone().get_field("activated").default(r.expr(false)).eq(false)
                        .and(document.clone().get_field("used").eq(false)),
                    document.clone().merge(r.expr(json!({
                        "token_hash": token_hash,
                        "revoked": false,
                        "created_at": Utc::now().timestamp()
                    }))),
                    document
                ))
            )
        });
        let result = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(TABLE).get(ornament_number).update(update)
        ).await?.ok_or_else(|| anyhow!("missing invitation reissue result"))?;
        if result.errors != 0 {
            bail!("invitation reissue failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(result.replaced == 1)
    }

    pub async fn revoke_invitation(&self, ornament_number: i16) -> Result<bool> {
        let update = reql::func!(|document| {
            r.branch(
                r.args((
                    document.clone().get_field("activated").default(r.expr(false)).eq(false)
                        .and(document.clone().get_field("used").eq(false)),
                    document.clone().merge(r.expr(json!({"revoked": true}))),
                    document
                ))
            )
        });
        let result = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(TABLE).get(ornament_number).update(update)
        ).await?.ok_or_else(|| anyhow!("missing invitation revoke result"))?;
        if result.errors != 0 {
            bail!("invitation revoke failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(result.replaced == 1)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthSession {
    pub id: String,
    pub subject_id: String,
    pub role: String,
    pub csrf_hash: String,
    pub created_at: i64,
    pub expires_at: i64,
    #[serde(default)]
    pub revoked: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ParticipantSummary {
    pub ornament_number: i16,
    pub activated: bool,
    pub submitted: bool,
    pub revoked: bool,
    pub display_name: Option<String>,
    pub created_at: i64,
    pub used_at: Option<i64>,
}
impl Store {
    pub async fn activate_invitation(&self, token_hash: &str) -> Result<Option<i16>> {
        let update = reql::func!(|document| {
            r.branch(r.args((
                document.clone().get_field("activated").default(r.expr(false)).eq(false)
                    .and(document.clone().get_field("revoked").default(r.expr(false)).eq(false))
                    .and(document.clone().get_field("used").eq(false)),
                document.clone().merge(r.expr(json!({
                    "activated": true, "activated_at": Utc::now().timestamp()
                }))),
                document
            )))
        });
        let query = r.db(self.database.as_str()).table(TABLE)
            .get_all(r.args((token_hash, token_index_options())))
            .limit(1).update(update);
        let result = self.first::<WriteResult>(query).await?
            .ok_or_else(|| anyhow!("missing invitation activation result"))?;
        if result.errors != 0 { bail!("invitation activation failed: {}", result.first_error.unwrap_or_default()); }
        if result.replaced != 1 { return Ok(None); }
        let mut stream = r.db(self.database.as_str()).table(TABLE)
            .get_all(r.args((token_hash, token_index_options())))
            .limit(1).run::<_, Ornament>(&self.session);
        let item = stream.try_next().await?.ok_or_else(|| anyhow!("activated invitation disappeared"))?;
        Ok(Some(item.ornament_number))
    }
    pub async fn create_auth_session(&self, session: &AuthSession) -> Result<()> {
        let result = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(SESSIONS_TABLE).insert(session)
        ).await?.ok_or_else(|| anyhow!("missing session insert result"))?;
        if result.errors != 0 || result.inserted != 1 {
            bail!("session insert failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(())
    }
    pub async fn get_auth_session(&self, id: &str) -> Result<Option<AuthSession>> {
        self.first(r.db(self.database.as_str()).table(SESSIONS_TABLE).get(id)).await
    }
    pub async fn revoke_auth_session(&self, id: &str) -> Result<()> {
        let _: Option<Value> = self.first(
            r.db(self.database.as_str()).table(SESSIONS_TABLE).get(id)
                .update(json!({"revoked": true}))
        ).await?;
        Ok(())
    }
    pub async fn list_participants(&self) -> Result<Vec<ParticipantSummary>> {
        Ok(self.list_ornaments().await?.into_iter().map(|o| ParticipantSummary {
            ornament_number: o.ornament_number, activated: o.activated, submitted: o.used,
            revoked: o.revoked, display_name: o.display_name, created_at: o.created_at,
            used_at: o.used_at,
        }).collect())
    }
    pub async fn claim_greeting(&self, number: i16, display_name: &str, message: &str) -> Result<bool> {
        let update = reql::func!(|document| {
            r.branch(r.args((
                document.clone().get_field("activated").default(r.expr(false))
                    .and(document.clone().get_field("revoked").default(r.expr(false)).eq(false))
                    .and(document.clone().get_field("used").eq(false)),
                document.clone().merge(r.expr(json!({
                    "used": true, "display_name": display_name, "message": message,
                    "used_at": Utc::now().timestamp()
                }))),
                document
            )))
        });
        let result = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(TABLE).get(number).update(update)
        ).await?.ok_or_else(|| anyhow!("missing greeting claim result"))?;
        if result.errors != 0 { bail!("greeting claim failed: {}", result.first_error.unwrap_or_default()); }
        Ok(result.replaced == 1)
    }
}

impl Store {
    pub async fn rollback_invitation_activation(&self, token_hash: &str) -> Result<()> {
        let _: Option<Value> = self.first(
            r.db(self.database.as_str()).table(TABLE)
                .get_all(r.args((token_hash, token_index_options())))
                .limit(1)
                .update(json!({"activated": false}))
        ).await?;
        Ok(())
    }
}


#[cfg(test)]
mod campaign_migration_compat_tests {
    use super::Ornament;

    #[test]
    fn legacy_ornament_without_campaign_id_remains_deserializable() {
        let ornament: Ornament = serde_json::from_value(serde_json::json!({
            "id": 1,
            "ornament_number": 1,
            "token_hash": "hash",
            "used": false,
            "activated": false,
            "revoked": false,
            "display_name": null,
            "message": null,
            "created_at": 1,
            "used_at": null
        })).expect("legacy ornament should deserialize");
        assert_eq!(ornament.campaign_id, None);
    }

    #[test]
    fn campaign_scoped_ornament_preserves_campaign_id() {
        let ornament: Ornament = serde_json::from_value(serde_json::json!({
            "id": 1,
            "ornament_number": 1,
            "campaign_id": "legacy-default",
            "token_hash": "hash",
            "used": false,
            "activated": false,
            "revoked": false,
            "display_name": null,
            "message": null,
            "created_at": 1,
            "used_at": null
        })).expect("campaign-scoped ornament should deserialize");
        assert_eq!(ornament.campaign_id.as_deref(), Some("legacy-default"));
    }
}
