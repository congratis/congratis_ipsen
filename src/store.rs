use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use futures::TryStreamExt;
use reql::{cmd::connect::Options, r, Session};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, time::Duration};

const TABLE: &str = "ornaments";
const TOKEN_INDEX: &str = "token_hash";

fn token_index_options() -> reql::cmd::get_all::Options {
    let mut options = reql::cmd::get_all::Options::default();
    options.index = Some(TOKEN_INDEX.into());
    options
}

#[derive(Clone)]
pub struct Store {
    session: Session,
    database: String,
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
    unchanged: u64,
    #[serde(default)]
    skipped: u64,
    #[serde(default)]
    errors: u64,
    first_error: Option<String>,
}

impl Store {
    pub async fn connect() -> Result<Self> {
        let host = env::var("RETHINKDB_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port: u16 = env::var("RETHINKDB_PORT")
            .unwrap_or_else(|_| "28015".into())
            .parse()
            .context("RETHINKDB_PORT must be a valid TCP port")?;
        let user = env::var("RETHINKDB_USER").unwrap_or_else(|_| "admin".into());
        let password = env::var("RETHINKDB_PASSWORD").unwrap_or_default();
        let database = env::var("RETHINKDB_DB").unwrap_or_else(|_| "congratis".into());

        let options = Options::new()
            .host(host)
            .port(port)
            .user(user)
            .password(password);
        let session = tokio::time::timeout(Duration::from_secs(10), r.connect(options))
            .await
            .context("timed out connecting to RethinkDB")??;

        let store = Self { session, database };
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

        let _: Option<Value> = self
            .first(
                r.db(self.database.as_str())
                    .table(TABLE)
                    .index_wait(TOKEN_INDEX),
            )
            .await
            .context("waiting for token_hash index")?;

        Ok(())
    }

    pub async fn list_ornaments(&self) -> Result<Vec<Ornament>> {
        let mut stream = r
            .db(self.database.as_str())
            .table(TABLE)
            .order_by("ornament_number")
            .run::<_, Ornament>(&self.session);

        let mut ornaments = Vec::new();
        while let Some(ornament) = stream.try_next().await? {
            ornaments.push(ornament);
        }
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
            if !(1..=20).contains(number) || !numbers.insert(*number) {
                bail!("invitation ornament numbers must be unique values between 1 and 20");
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
                document.get_field("used").eq(false),
                document.merge(r.expr(json!({
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
