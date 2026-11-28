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
const LIKES_TABLE: &str = "greeting_likes";
pub const LEGACY_CAMPAIGN_ID: &str = "legacy-default";
const INDEX_PROBE_TOKEN: &str = "__congratis_index_probe__";

fn active_campaign_status() -> String { "active".to_owned() }

fn validate_campaign_deletion(campaign_id: &str, campaign_name: &str, confirmation: &str) -> Result<()> {
    if campaign_id == LEGACY_CAMPAIGN_ID {
        bail!("the legacy campaign cannot be deleted");
    }
    if campaign_name != confirmation {
        bail!("campaign name confirmation does not match exactly");
    }
    Ok(())
}

fn campaign_ornament_key(campaign_id: &str, ornament_number: i16) -> String {
    format!("{campaign_id}:{ornament_number}")
}

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
    pub campaign_id: String,
    pub ornament_number: i16,
    pub display_name: String,
    pub surname: Option<String>,
    pub first_name: Option<String>,
    pub avatar_filename: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ornament {
    pub id: Value,
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
    #[serde(default)]
    pub surname: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub avatar_filename: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub liked_by: Vec<i16>,
    pub created_at: i64,
    pub used_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Campaign {
    pub id: String,
    pub name: String,
    #[serde(default = "active_campaign_status")]
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct GreetingLike {
    id: String,
    campaign_id: String,
    target_number: i16,
    voter_number: i16,
    created_at: i64,
}

#[derive(Debug, Default, Deserialize)]
struct WriteResult {
    #[serde(default)]
    inserted: u64,
    #[serde(default)]
    replaced: u64,
    #[serde(default)]
    deleted: u64,
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

        let tables = self.first::<Vec<String>>(
            r.db(self.database.as_str()).table_list()
        ).await?.ok_or_else(|| anyhow!("RethinkDB returned no table list"))?;
        if !tables.iter().any(|name| name == LIKES_TABLE) {
            let _: Option<Value> = self.first(
                r.db(self.database.as_str()).table_create(LIKES_TABLE)
            ).await.context("creating greeting_likes table")?;
        }

        self.migrate_legacy_campaign().await?;
        Ok(())
    }

    /// Additively assigns all pre-multi-campaign records to one stable legacy campaign.
    /// It never rewrites existing campaign IDs and is safe to run on every startup.
    async fn migrate_legacy_campaign(&self) -> Result<()> {
        let ornaments = self.list_ornaments().await?;

        // Read sessions before writing so a failed earlier run can be resumed even
        // when all ornaments were already migrated.
        let mut sessions_stream = r
            .db(self.database.as_str())
            .table(SESSIONS_TABLE)
            .run::<_, Value>(&self.session);
        let mut sessions = Vec::new();
        while let Some(session) = sessions_stream.try_next().await? {
            sessions.push(session);
        }
        drop(sessions_stream);

        let mut needs_legacy_campaign = ornaments.iter().any(|item| {
            item.campaign_id.as_deref().is_none_or(str::is_empty)
        });

        for session in &sessions {
            if session.get("role").and_then(Value::as_str) != Some("participant")
                || session.get("revoked").and_then(Value::as_bool) == Some(true)
                || session.get("campaign_id").and_then(Value::as_str).is_some_and(|id| !id.is_empty())
            {
                continue;
            }

            let subject = session.get("subject_id").and_then(Value::as_str)
                .and_then(|value| value.parse::<i16>().ok());
            if let Some(ornament) = subject.and_then(|number| {
                ornaments.iter().find(|item| item.ornament_number == number)
            }) {
                if ornament.campaign_id.as_deref().is_none_or(str::is_empty)
                    || ornament.campaign_id.as_deref() == Some(LEGACY_CAMPAIGN_ID)
                {
                    needs_legacy_campaign = true;
                }
            }
        }

        if needs_legacy_campaign {
            let existing: Option<Value> = self
                .first(r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(LEGACY_CAMPAIGN_ID))
                .await?;
            if existing.is_none() {
                let now = Utc::now().timestamp();
                let result = self
                    .first::<WriteResult>(
                        r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).insert(json!({
                            "id": LEGACY_CAMPAIGN_ID,
                            "name": "Существующая кампания",
                            "status": "active",
                            "created_at": now,
                            "updated_at": now
                        })),
                    )
                    .await?
                    .ok_or_else(|| anyhow!("missing legacy campaign insert result"))?;
                if result.errors != 0 || result.inserted != 1 {
                    let found: Option<Value> = self
                        .first(r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(LEGACY_CAMPAIGN_ID))
                        .await?;
                    if found.is_none() {
                        bail!("legacy campaign creation failed: {}", result.first_error.unwrap_or_default());
                    }
                }
            }
        }

        for ornament in ornaments.iter().filter(|item| {
            item.campaign_id.as_deref().is_none_or(str::is_empty)
        }) {
            let result = self
                .first::<WriteResult>(
                    r.db(self.database.as_str())
                        .table(TABLE)
                        .get(ornament.id.clone())
                        .update(json!({"campaign_id": LEGACY_CAMPAIGN_ID})),
                )
                .await?
                .ok_or_else(|| anyhow!("missing legacy ornament migration result"))?;
            if result.errors != 0 {
                bail!(
                    "legacy ornament migration failed for {}: {}",
                    ornament.id,
                    result.first_error.unwrap_or_default()
                );
            }
        }

        for session in sessions {
            if session.get("role").and_then(Value::as_str) != Some("participant")
                || session.get("revoked").and_then(Value::as_bool) == Some(true)
                || session.get("campaign_id").and_then(Value::as_str).is_some_and(|id| !id.is_empty())
            {
                continue;
            }

            let id = session.get("id").and_then(Value::as_str)
                .ok_or_else(|| anyhow!("participant session without an ID during campaign migration"))?
                .to_owned();
            let subject = session.get("subject_id").and_then(Value::as_str)
                .and_then(|value| value.parse::<i16>().ok());
            let matching_ornament = subject.and_then(|number| {
                ornaments.iter().find(|item| item.ornament_number == number)
            });

            let update = if let Some(ornament) = matching_ornament {
                let campaign_id = ornament.campaign_id.as_deref()
                    .filter(|value| !value.is_empty())
                    .unwrap_or(LEGACY_CAMPAIGN_ID);
                json!({"campaign_id": campaign_id})
            } else {
                // A session that cannot be tied to an existing invitation must
                // never be assigned to an arbitrary campaign.
                json!({"revoked": true})
            };

            let result = self
                .first::<WriteResult>(
                    r.db(self.database.as_str())
                        .table(SESSIONS_TABLE)
                        .get(id.as_str())
                        .update(update),
                )
                .await?
                .ok_or_else(|| anyhow!("missing participant session migration result"))?;
            if result.errors != 0 {
                bail!("participant session migration failed: {}", result.first_error.unwrap_or_default());
            }
        }

        let migrated = self.list_ornaments().await?;
        if migrated.iter().any(|item| {
            item.campaign_id.as_deref().is_none_or(str::is_empty)
        }) {
            bail!("campaign migration verification failed: an invitation has no campaign assignment");
        }

        let mut campaign_stream = r
            .db(self.database.as_str())
            .table(CAMPAIGNS_TABLE)
            .run::<_, Value>(&self.session);
        let mut campaign_ids = std::collections::HashSet::new();
        while let Some(campaign) = campaign_stream.try_next().await? {
            if let Some(id) = campaign.get("id").and_then(Value::as_str) {
                campaign_ids.insert(id.to_owned());
            }
        }
        drop(campaign_stream);

        if migrated.iter().any(|item| {
            item.campaign_id.as_ref().is_some_and(|id| !campaign_ids.contains(id))
        }) {
            bail!("campaign migration verification failed: an invitation references a missing campaign");
        }

        let mut verification_stream = r
            .db(self.database.as_str())
            .table(SESSIONS_TABLE)
            .run::<_, Value>(&self.session);
        while let Some(session) = verification_stream.try_next().await? {
            if session.get("role").and_then(Value::as_str) != Some("participant")
                || session.get("revoked").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            let campaign_id = session.get("campaign_id").and_then(Value::as_str);
            if campaign_id.is_none_or(|id| !campaign_ids.contains(id)) {
                bail!("campaign migration verification failed: an active participant session has no valid campaign");
            }
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

    pub async fn list_campaigns(&self) -> Result<Vec<Campaign>> {
        let mut stream = r
            .db(self.database.as_str())
            .table(CAMPAIGNS_TABLE)
            .run::<_, Campaign>(&self.session);
        let mut campaigns = Vec::new();
        while let Some(campaign) = stream.try_next().await? {
            campaigns.push(campaign);
        }
        campaigns.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.name.cmp(&b.name)));
        Ok(campaigns)
    }

    pub async fn get_campaign(&self, campaign_id: &str) -> Result<Option<Campaign>> {
        let value: Option<Value> = self
            .first(r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(campaign_id))
            .await?;
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(value) => Ok(Some(serde_json::from_value(value)?)),
        }
    }

    pub async fn create_campaign(&self, campaign_id: &str, name: &str) -> Result<Campaign> {
        let name = name.trim();
        if campaign_id.trim().is_empty() || name.is_empty() || name.chars().count() > 100 {
            bail!("campaign ID and a campaign name of 1–100 characters are required");
        }
        if self.get_campaign(campaign_id).await?.is_some() {
            bail!("campaign already exists");
        }
        let now = Utc::now().timestamp();
        let campaign = Campaign {
            id: campaign_id.to_owned(),
            name: name.to_owned(),
            status: active_campaign_status(),
            created_at: now,
            updated_at: now,
        };
        let result = self
            .first::<WriteResult>(
                r.db(self.database.as_str())
                    .table(CAMPAIGNS_TABLE)
                    .insert(&campaign),
            )
            .await?
            .ok_or_else(|| anyhow!("missing campaign insert result"))?;
        if result.errors != 0 || result.inserted != 1 {
            bail!("campaign insert failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(campaign)
    }

    /// Deletes a non-legacy campaign. The campaign is marked as deleting first;
    /// all cleanup steps are scoped and safe to retry after a partial failure.
    pub async fn delete_campaign(&self, campaign_id: &str, confirmation: &str) -> Result<bool> {
        let Some(campaign) = self.get_campaign(campaign_id).await? else {
            return Ok(false);
        };
        validate_campaign_deletion(campaign_id, &campaign.name, confirmation)?;

        let now = Utc::now().timestamp();
        let marked = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(campaign_id)
                .update(json!({"status": "deleting", "updated_at": now}))
        ).await?.ok_or_else(|| anyhow!("missing campaign deletion marker result"))?;
        if marked.errors != 0 {
            bail!("campaign deletion marker failed: {}", marked.first_error.unwrap_or_default());
        }

        // Remove only invitation documents that still carry this campaign ID.
        for ornament in self.list_campaign_ornaments(campaign_id).await? {
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(TABLE).get(ornament.id).delete(())
            ).await?.ok_or_else(|| anyhow!("missing campaign invitation deletion result"))?;
            if result.errors != 0 {
                bail!("campaign invitation deletion failed: {}", result.first_error.unwrap_or_default());
            }
        }

        // Remove likes after the greeting documents have been deleted.
        for like in self.list_campaign_like_rows(campaign_id).await? {
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(LIKES_TABLE).get(like.id).delete(())
            ).await?.ok_or_else(|| anyhow!("missing campaign like deletion result"))?;
            if result.errors != 0 {
                bail!("campaign like deletion failed: {}", result.first_error.unwrap_or_default());
            }
        }

        // Delete participant sessions only; administrator sessions have no campaign ID.
        for _ in 0..2 {
            let mut stream = r.db(self.database.as_str()).table(SESSIONS_TABLE).run::<_, Value>(&self.session);
            let mut session_ids = Vec::new();
            while let Some(session) = stream.try_next().await? {
                if session.get("role").and_then(Value::as_str) == Some("participant")
                    && session.get("campaign_id").and_then(Value::as_str) == Some(campaign_id)
                {
                    if let Some(id) = session.get("id").and_then(Value::as_str) {
                        session_ids.push(id.to_owned());
                    }
                }
            }
            drop(stream);
            for id in session_ids {
                let result = self.first::<WriteResult>(
                    r.db(self.database.as_str()).table(SESSIONS_TABLE).get(id.as_str()).delete(())
                ).await?.ok_or_else(|| anyhow!("missing participant session deletion result"))?;
                if result.errors != 0 {
                    bail!("participant session deletion failed: {}", result.first_error.unwrap_or_default());
                }
            }
        }

        // Delete the campaign record last so an interrupted cleanup can be retried.
        let result = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(CAMPAIGNS_TABLE).get(campaign_id).delete(())
        ).await?.ok_or_else(|| anyhow!("missing campaign record deletion result"))?;
        if result.errors != 0 {
            bail!("campaign record deletion failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(result.deleted == 1)
    }

    pub async fn list_campaign_ornaments(&self, campaign_id: &str) -> Result<Vec<Ornament>> {
        let mut stream = r
            .db(self.database.as_str())
            .table(TABLE)
            .filter(reql::func!(|document| document.get_field("campaign_id").eq(campaign_id)))
            .run::<_, Ornament>(&self.session);
        let mut ornaments = Vec::new();
        while let Some(ornament) = stream.try_next().await? {
            ornaments.push(ornament);
        }
        ornaments.sort_by_key(|item| item.ornament_number);
        Ok(ornaments)
    }

    /// Inserts invitations under deterministic campaign-scoped primary keys.
    /// A slot collision or a globally duplicated token hash is rejected.
    pub async fn insert_campaign_invitations(
        &self,
        campaign_id: &str,
        invitations: &[(i16, String)],
    ) -> Result<()> {
        if self.get_campaign(campaign_id).await?.is_none_or(|campaign| campaign.status != "active") {
            bail!("campaign does not exist or is not active");
        }
        if invitations.is_empty() {
            bail!("at least one invitation is required");
        }

        let existing = self.list_campaign_ornaments(campaign_id).await?;
        let occupied: std::collections::HashSet<i16> =
            existing.iter().map(|item| item.ornament_number).collect();
        let all_ornaments = self.list_ornaments().await?;
        let existing_hashes: std::collections::HashSet<String> =
            all_ornaments.into_iter().map(|item| item.token_hash).collect();
        let mut numbers = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();

        for (number, token_hash) in invitations {
            if !(1..=35).contains(number)
                || occupied.contains(number)
                || !numbers.insert(*number)
            {
                bail!("invitation slots must be unique unused values between 1 and 35");
            }
            if existing_hashes.contains(token_hash) || !hashes.insert(token_hash) {
                bail!("duplicate invitation token hash");
            }
        }

        let created_at = Utc::now().timestamp();
        let documents: Vec<Value> = invitations
            .iter()
            .map(|(number, token_hash)| {
                json!({
                    "id": campaign_ornament_key(campaign_id, *number),
                    "ornament_number": number,
                    "campaign_id": campaign_id,
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
            .ok_or_else(|| anyhow!("missing campaign invitation insert result"))?;
        if result.errors != 0 || result.inserted != invitations.len() as u64 {
            bail!(
                "campaign invitation insert incomplete: inserted={}, errors={}, first_error={:?}",
                result.inserted,
                result.errors,
                result.first_error
            );
        }
        Ok(())
    }

    pub async fn list_campaign_participants(
        &self,
        campaign_id: &str,
    ) -> Result<Vec<ParticipantSummary>> {
        let ornaments = self.list_campaign_ornaments(campaign_id).await?;
        let likes = self.list_campaign_like_rows(campaign_id).await?;
        Ok(ornaments.into_iter().map(|item| ParticipantSummary {
            ornament_number: item.ornament_number,
            activated: item.activated,
            submitted: item.used,
            revoked: item.revoked,
            display_name: item.display_name,
            surname: item.surname,
            first_name: item.first_name,
            avatar_filename: item.avatar_filename,
            created_at: item.created_at,
            used_at: item.used_at,
            like_count: likes.iter().filter(|like| like.target_number == item.ornament_number).count(),
        }).collect())
    }

    pub async fn reissue_campaign_invitation(
        &self,
        campaign_id: &str,
        ornament_number: i16,
        token_hash: &str,
    ) -> Result<bool> {
        let id = campaign_ornament_key(campaign_id, ornament_number);
        let update = reql::func!(|document| {
            r.branch(
                r.args((
                    document.clone().get_field("campaign_id").eq(campaign_id)
                        .and(document.clone().get_field("activated").default(r.expr(false)).eq(false))
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
        let result = self
            .first::<WriteResult>(
                r.db(self.database.as_str())
                    .table(TABLE)
                    .get(id)
                    .update(update),
            )
            .await?
            .ok_or_else(|| anyhow!("missing campaign invitation reissue result"))?;
        if result.errors != 0 {
            bail!("campaign invitation reissue failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(result.replaced == 1)
    }

    pub async fn revoke_campaign_invitation(
        &self,
        campaign_id: &str,
        ornament_number: i16,
    ) -> Result<bool> {
        let id = campaign_ornament_key(campaign_id, ornament_number);
        let update = reql::func!(|document| {
            r.branch(
                r.args((
                    document.clone().get_field("campaign_id").eq(campaign_id)
                        .and(document.clone().get_field("activated").default(r.expr(false)).eq(false))
                        .and(document.clone().get_field("used").eq(false)),
                    document.clone().merge(r.expr(json!({"revoked": true}))),
                    document
                ))
            )
        });
        let result = self
            .first::<WriteResult>(
                r.db(self.database.as_str())
                    .table(TABLE)
                    .get(id)
                    .update(update),
            )
            .await?
            .ok_or_else(|| anyhow!("missing campaign invitation revoke result"))?;
        if result.errors != 0 {
            bail!("campaign invitation revoke failed: {}", result.first_error.unwrap_or_default());
        }
        Ok(result.replaced == 1)
    }

    pub async fn list_greetings(&self) -> Result<Vec<Ornament>> {
        Ok(self.list_campaign_greetings(LEGACY_CAMPAIGN_ID).await?)
    }

    async fn list_campaign_like_rows(&self, campaign_id: &str) -> Result<Vec<GreetingLike>> {
        let mut stream = r.db(self.database.as_str()).table(LIKES_TABLE)
            .filter(reql::func!(|document| document.get_field("campaign_id").eq(campaign_id)))
            .run::<_, GreetingLike>(&self.session);
        let mut likes = Vec::new();
        while let Some(like) = stream.try_next().await? {
            likes.push(like);
        }
        Ok(likes)
    }

    pub async fn list_campaign_greetings(&self, campaign_id: &str) -> Result<Vec<Ornament>> {
        let mut ornaments = self.list_campaign_ornaments(campaign_id).await?;
        let likes = self.list_campaign_like_rows(campaign_id).await?;
        for ornament in &mut ornaments {
            ornament.liked_by = likes.iter()
                .filter(|like| like.target_number == ornament.ornament_number)
                .map(|like| like.voter_number)
                .collect();
        }
        Ok(ornaments.into_iter().filter(|ornament| ornament.used).collect())
    }

    /// Toggles a campaign-scoped like using a deterministic primary key.
    /// The likes table guarantees at most one record per voter and greeting.
    pub async fn toggle_greeting_like(
        &self,
        campaign_id: &str,
        voter_number: i16,
        target_number: i16,
    ) -> Result<Option<(bool, usize)>> {
        if voter_number == target_number {
            return Ok(None);
        }
        let ornament_id = if campaign_id == LEGACY_CAMPAIGN_ID {
            json!(target_number)
        } else {
            json!(campaign_ornament_key(campaign_id, target_number))
        };
        let target: Option<Ornament> = self.first(
            r.db(self.database.as_str()).table(TABLE).get(ornament_id)
        ).await?;
        let Some(target) = target else { return Ok(None); };
        if !target.used || target.campaign_id.as_deref().unwrap_or(LEGACY_CAMPAIGN_ID) != campaign_id {
            return Ok(None);
        }

        let like_id = format!("{campaign_id}:{target_number}:{voter_number}");
        let existing: Option<GreetingLike> = self.first(
            r.db(self.database.as_str()).table(LIKES_TABLE).get(like_id.clone())
        ).await?;
        let liked = if existing.is_some() {
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(LIKES_TABLE).get(like_id).delete(())
            ).await?.ok_or_else(|| anyhow!("missing greeting unlike result"))?;
            if result.errors != 0 {
                bail!("greeting unlike failed: {}", result.first_error.unwrap_or_default());
            }
            false
        } else {
            let result = self.first::<WriteResult>(
                r.db(self.database.as_str()).table(LIKES_TABLE).insert(json!({
                    "id": like_id,
                    "campaign_id": campaign_id,
                    "target_number": target_number,
                    "voter_number": voter_number,
                    "created_at": Utc::now().timestamp()
                }))
            ).await?.ok_or_else(|| anyhow!("missing greeting like result"))?;
            if result.errors != 0 {
                // Another request may have inserted the same deterministic key
                // after our read. Treat that as already liked, not as a 500.
                let raced: Option<GreetingLike> = self.first(
                    r.db(self.database.as_str()).table(LIKES_TABLE).get(format!("{campaign_id}:{target_number}:{voter_number}"))
                ).await?;
                if raced.is_none() {
                    bail!("greeting like failed: {}", result.first_error.unwrap_or_default());
                }
            }
            true
        };
        let like_count = self.list_campaign_like_rows(campaign_id).await?
            .iter().filter(|like| like.target_number == target_number).count();
        Ok(Some((liked, like_count)))
    }

    pub async fn insert_invitations(&self, invitations: &[(i16, String)]) -> Result<()> {
        // The legacy campaign is a stable system scope even on a fresh database
        // where no pre-existing ornaments were present during startup migration.
        if self.get_campaign(LEGACY_CAMPAIGN_ID).await?.is_none() {
            self.create_campaign(LEGACY_CAMPAIGN_ID, "Существующая кампания").await?;
        }
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

            let campaign_id = ornament.campaign_id.clone()
                .unwrap_or_else(|| LEGACY_CAMPAIGN_ID.to_owned());
            let _ = sender.send(GreetingEvent {
                campaign_id,
                ornament_number: ornament.ornament_number,
                display_name,
                surname: ornament.surname,
                first_name: ornament.first_name,
                avatar_filename: ornament.avatar_filename,
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
    #[serde(default)]
    pub campaign_id: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ParticipantSummary {
    pub ornament_number: i16,
    pub activated: bool,
    pub submitted: bool,
    pub revoked: bool,
    pub display_name: Option<String>,
    #[serde(default)]
    pub surname: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub avatar_filename: Option<String>,
    pub created_at: i64,
    pub used_at: Option<i64>,
    #[serde(default)]
    pub like_count: usize,
}
impl Store {
    pub async fn activate_invitation(&self, token_hash: &str) -> Result<Option<(i16, String)>> {
        let mut existing = r.db(self.database.as_str()).table(TABLE)
            .get_all(r.args((token_hash, token_index_options())))
            .limit(1).run::<_, Ornament>(&self.session);
        let Some(invitation) = existing.try_next().await? else {
            return Ok(None);
        };
        drop(existing);
        let campaign_id = invitation.campaign_id.as_deref().unwrap_or(LEGACY_CAMPAIGN_ID);
        if self.get_campaign(campaign_id).await?.is_none_or(|campaign| campaign.status != "active") {
            return Ok(None);
        }

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
        let campaign_id = item.campaign_id.unwrap_or_else(|| LEGACY_CAMPAIGN_ID.to_owned());
        Ok(Some((item.ornament_number, campaign_id)))
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
        let value: Option<Value> = self
            .first(r.db(self.database.as_str()).table(SESSIONS_TABLE).get(id))
            .await?;
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(value) => Ok(Some(serde_json::from_value(value)?)),
        }
    }
    pub async fn revoke_auth_session(&self, id: &str) -> Result<()> {
        let _: Option<Value> = self.first(
            r.db(self.database.as_str()).table(SESSIONS_TABLE).get(id)
                .update(json!({"revoked": true}))
        ).await?;
        Ok(())
    }
    pub async fn list_participants(&self) -> Result<Vec<ParticipantSummary>> {
        self.list_campaign_participants(LEGACY_CAMPAIGN_ID).await
    }
    pub async fn claim_greeting(&self, campaign_id: &str, number: i16, surname: &str, first_name: &str, avatar_filename: Option<&str>, message: &str) -> Result<bool> {
        if self.get_campaign(campaign_id).await?.is_none_or(|campaign| campaign.status != "active") {
            return Ok(false);
        }
        let update = reql::func!(|document| {
            r.branch(r.args((
                document.clone().get_field("activated").default(r.expr(false))
                    .and(document.clone().get_field("revoked").default(r.expr(false)).eq(false))
                    .and(document.clone().get_field("used").eq(false)),
                document.clone().merge(r.expr(json!({
                    "used": true,
                    "surname": surname,
                    "first_name": first_name,
                    "display_name": format!("{} {}", surname, first_name),
                    "avatar_filename": avatar_filename,
                    "message": message,
                    "used_at": Utc::now().timestamp()
                }))),
                document
            )))
        });
        let id = if campaign_id == LEGACY_CAMPAIGN_ID {
            json!(number)
        } else {
            json!(campaign_ornament_key(campaign_id, number))
        };
        let result = self.first::<WriteResult>(
            r.db(self.database.as_str()).table(TABLE).get(id).update(update)
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
mod campaign_storage_model_tests {
    use super::{campaign_ornament_key, Ornament};

    fn ornament_json(id: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "ornament_number": 1,
            "campaign_id": "campaign-a",
            "token_hash": "hash",
            "used": false,
            "activated": false,
            "revoked": false,
            "display_name": null,
            "message": null,
            "created_at": 1,
            "used_at": null
        })
    }

    #[test]
    fn legacy_numeric_primary_key_remains_compatible() {
        let ornament: Ornament = serde_json::from_value(ornament_json(serde_json::json!(1)))
            .expect("legacy numeric ID should deserialize");
        assert_eq!(ornament.id, serde_json::json!(1));
    }

    #[test]
    fn campaign_scoped_string_primary_key_deserializes() {
        let ornament: Ornament =
            serde_json::from_value(ornament_json(serde_json::json!("campaign-a:1")))
                .expect("campaign-scoped string ID should deserialize");
        assert_eq!(ornament.id, serde_json::json!("campaign-a:1"));
    }

    #[test]
    fn identical_ornament_numbers_have_distinct_campaign_keys() {
        assert_eq!(campaign_ornament_key("campaign-a", 1), "campaign-a:1");
        assert_eq!(campaign_ornament_key("campaign-b", 1), "campaign-b:1");
        assert_ne!(campaign_ornament_key("campaign-a", 1), campaign_ornament_key("campaign-b", 1));
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


#[cfg(test)]
mod campaign_deletion_validation_tests {
    use super::{validate_campaign_deletion, LEGACY_CAMPAIGN_ID};

    #[test]
    fn deletion_requires_exact_campaign_name() {
        assert!(validate_campaign_deletion("campaign-a", "Winter 2026", "Winter 2026").is_ok());
        assert!(validate_campaign_deletion("campaign-a", "Winter 2026", "winter 2026").is_err());
        assert!(validate_campaign_deletion("campaign-a", "Winter 2026", "Winter 2026 ").is_err());
    }

    #[test]
    fn legacy_campaign_is_never_deletable() {
        assert!(validate_campaign_deletion(LEGACY_CAMPAIGN_ID, "Существующая кампания", "Существующая кампания").is_err());
    }
}
