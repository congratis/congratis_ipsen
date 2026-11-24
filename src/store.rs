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

#[derive(Debug, Clone)]
pub enum RealtimeEvent {
    Greeting(GreetingEvent),
    Removed { ornament_number: i16 },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ornament {
    pub id: i16,
    pub ornament_number: i16,
    pub token_hash: String,
    pub used: bool,
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
        if !self.list_ornaments().await?.is_empty() {
            bail!("ornaments table is not empty; refusing to overwrite invitations");
        }
        self.insert_invitation_documents(invitations).await
    }

    /// Add invitations into currently free ornament slots without touching existing rows.
    pub async fn create_invitations(&self, invitations: &[(i16, String)]) -> Result<()> {
        let existing = self.list_ornaments().await?;
        let occupied: std::collections::HashSet<i16> =
            existing.into_iter().map(|item| item.ornament_number).collect();

        let mut numbers = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();
        for (number, token_hash) in invitations {
            if !(1..=20).contains(number) || occupied.contains(number) || !numbers.insert(*number) {
                bail!("invitation ornament numbers must be unique free values between 1 and 20");
            }
            if !hashes.insert(token_hash) {
                bail!("duplicate invitation token hash in batch");
            }
        }
        self.insert_invitation_documents(invitations).await
    }

    async fn insert_invitation_documents(&self, invitations: &[(i16, String)]) -> Result<()> {
        let mut numbers = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();
        for (number, token_hash) in invitations {
            if !(1..=20).contains(number) || !numbers.insert(*number) {
                bail!("invitation ornament numbers must be unique values between 1 and 20");
            }
            if !hashes.insert(token_hash) {
                bail!("duplicate invitation token hash in batch");
            }
        }
        if invitations.is_empty() {
            bail!("at least one invitation is required");
        }

        let created_at = Utc::now().timestamp();
        let documents: Vec<Value> = invitations
            .iter()
            .map(|(number, token_hash)| {
                json!({
                    "id": number,
                    "ornament_number": number,
                    "token_hash": token_hash,
                    "used": false,
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

    /// Delete an invitation, or a submitted greeting after explicit admin confirmation.
    /// The conditional replace prevents a stale "unused" view from deleting a greeting
    /// that was submitted just before the admin action.
    pub async fn delete_ornament(&self, number: i16, confirm_used: bool) -> Result<bool> {
        if !(1..=20).contains(&number) {
            bail!("ornament number must be between 1 and 20");
        }
        let delete_if_allowed = reql::func!(|document| {
            r.branch(
                document.clone().get_field("used").eq(false).or(r.expr(confirm_used)),
                r.expr(Value::Null),
                document
            )
        });
        let result = self
            .first::<WriteResult>(
                r.db(self.database.as_str())
                    .table(TABLE)
                    .get(number)
                    .replace(delete_if_allowed),
            )
            .await?
            .ok_or_else(|| anyhow!("RethinkDB returned no ornament deletion result"))?;
        if result.errors != 0 {
            bail!(
                "RethinkDB ornament deletion failed: {}",
                result.first_error.unwrap_or_else(|| "unknown write error".into())
            );
        }
        Ok(result.deleted == 1)
    }

    pub async fn claim_invitation(
        &self,
        token_hash: &str,
        display_name: &str,
        message: &str,
    ) -> Result<Option<i16>> {
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

    pub async fn supervise_changefeed(self, sender: broadcast::Sender<RealtimeEvent>) {
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
        sender: &broadcast::Sender<RealtimeEvent>,
        ready: watch::Sender<bool>,
    ) -> Result<()> {
        let session = self.connection.connect().await?;
        self.consume_changefeed(&session, sender, Some(&ready)).await
    }

    async fn consume_changefeed(
        &self,
        session: &Session,
        sender: &broadcast::Sender<RealtimeEvent>,
        ready: Option<&watch::Sender<bool>>,
    ) -> Result<()> {
        let mut options = reql::cmd::changes::Options::default();
        options.include_initial = Some(true);
        options.include_states = Some(true);
        options.include_types = Some(true);

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
                continue;
            };
            if new_value.is_null() {
                if let Some(old_value) = change.get("old_val").filter(|value| !value.is_null()) {
                    if let Ok(ornament) = serde_json::from_value::<Ornament>(old_value.clone()) {
                        let _ = sender.send(RealtimeEvent::Removed {
                            ornament_number: ornament.ornament_number,
                        });
                    }
                }
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

            let _ = sender.send(RealtimeEvent::Greeting(GreetingEvent {
                ornament_number: ornament.ornament_number,
                display_name,
                message,
            }));
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


#[cfg(test)]
mod admin_invitation_tests {
    use super::Store;
    use anyhow::{ensure, Result};
    use std::env;
    use std::time::Duration;
    use tokio::sync::{broadcast, watch};

    #[tokio::test]
    async fn admin_invitation_create_and_delete_flow() -> Result<()> {
        if env::var("RETHINKDB_TESTS").ok().as_deref() != Some("1") {
            return Ok(());
        }

        let database = env::var("RETHINKDB_DB").unwrap_or_default();
        ensure!(
            database.ends_with("_ci"),
            "RETHINKDB_TESTS=1 is only allowed with a database ending in _ci"
        );

        let store = Store::connect().await?;
        // This test owns the isolated *_ci database and starts from an empty table.
        for ornament in store.list_ornaments().await? {
            store.delete_ornament(ornament.ornament_number, true).await?;
        }
        ensure!(store.list_ornaments().await?.is_empty());

        let (events, mut event_rx) = broadcast::channel(32);
        let (ready_tx, mut ready_rx) = watch::channel(false);
        let feed_store = store.clone();
        let feed_events = events.clone();
        tokio::spawn(async move {
            let _ = feed_store.subscribe_changefeed(&feed_events, ready_tx).await;
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            while !*ready_rx.borrow() {
                ready_rx.changed().await?;
            }
            Ok::<(), tokio::sync::watch::error::RecvError>(())
        })
        .await??;

        let unused_hash = "a".repeat(64);
        let used_hash = "b".repeat(64);
        store
            .create_invitations(&[(1, unused_hash.clone()), (2, used_hash.clone())])
            .await?;

        let created = store.list_ornaments().await?;
        ensure!(created.len() == 2);
        ensure!(created.iter().all(|item| !item.used));

        // An unused invitation can be removed without the used-confirmation flag.
        ensure!(store.delete_ornament(1, false).await?);

        // A submitted greeting cannot be removed by a stale admin view.
        ensure!(
            store
                .claim_invitation(&used_hash, "CI test", "Greeting for integration test")
                .await?
                == Some(2)
        );
        ensure!(!store.delete_ornament(2, false).await?);

        // Explicit confirmation permits deletion of the submitted greeting.
        ensure!(store.delete_ornament(2, true).await?);
        ensure!(store.list_ornaments().await?.is_empty());

        // The public SSE feed must tell connected clients to clear the deleted slot.
        let removal = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match event_rx.recv().await? {
                    super::RealtimeEvent::Removed { ornament_number: 2 } => break Ok(()),
                    _ => continue,
                }
            }
        })
        .await??;
        let _ = removal;
        drop(events);

        Ok(())
    }
}
