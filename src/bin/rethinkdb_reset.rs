use anyhow::{bail, Context, Result};
use futures::TryStreamExt;
use reql::{cmd::connect::Options, r};
use serde::Deserialize;
use std::{env, time::Duration};

#[derive(Debug, Default, Deserialize)]
struct DeleteResult {
    #[serde(default)]
    deleted: u64,
    #[serde(default)]
    errors: u64,
    first_error: Option<String>,
}

async fn first<T>(command: reql::Command, session: &reql::Session) -> Result<T>
where
    T: serde::de::DeserializeOwned + Unpin,
{
    let mut stream = command.run::<_, T>(session);
    stream
        .try_next()
        .await?
        .context("RethinkDB returned no result")
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("--confirm-database") {
        bail!("usage: rethinkdb-reset --confirm-database <database-name>");
    }
    let confirmed_database = args
        .next()
        .context("missing database name after --confirm-database")?;
    if args.next().is_some() {
        bail!("unexpected extra arguments");
    }

    let database = env::var("RETHINKDB_DB").unwrap_or_else(|_| "congratis".into());
    if confirmed_database != database {
        bail!(
            "database confirmation mismatch: configured database is {:?}, but {:?} was confirmed",
            database,
            confirmed_database
        );
    }

    let host = env::var("RETHINKDB_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = env::var("RETHINKDB_PORT")
        .unwrap_or_else(|_| "28015".into())
        .parse()
        .context("RETHINKDB_PORT must be a valid TCP port")?;
    let user = env::var("RETHINKDB_USER").unwrap_or_else(|_| "admin".into());
    let password = env::var("RETHINKDB_PASSWORD").unwrap_or_default();

    let options = Options::new()
        .host(host)
        .port(port)
        .user(user)
        .password(password);
    let session = tokio::time::timeout(Duration::from_secs(10), r.connect(options))
        .await
        .context("timed out connecting to RethinkDB")??;

    let databases: Vec<String> = first(r.db_list(), &session).await?;
    if !databases.iter().any(|name| name == &database) {
        bail!("database {database:?} does not exist; refusing to create or reset anything");
    }

    let tables: Vec<String> =
        first(r.db(database.as_str()).table_list(), &session).await?;
    for table in ["ornaments", "auth_sessions"] {
        if !tables.iter().any(|name| name == table) {
            bail!("required table {database}.{table} does not exist; nothing was cleared");
        }
    }

    println!("Connected to database: {database}");
    for table in ["ornaments", "auth_sessions"] {
        let query = r.db(database.as_str()).table(table);
        let before: u64 = first(query.clone().count(), &session).await?;
        let result: DeleteResult = first(query.delete(), &session).await?;

        if result.errors != 0 {
            bail!(
                "deleting {database}.{table} failed ({} errors): {}",
                result.errors,
                result.first_error.unwrap_or_else(|| "unknown error".into())
            );
        }
        if result.deleted != before {
            bail!(
                "unexpected delete count for {database}.{table}: had {before}, deleted {}",
                result.deleted
            );
        }

        let remaining: u64 = first(
            r.db(database.as_str()).table(table).count(),
            &session,
        )
        .await?;
        if remaining != 0 {
            bail!("{database}.{table} is not empty after reset: {remaining} rows remain");
        }

        println!("{table}: {before} records found, {} deleted, verified empty", result.deleted);
    }

    println!("Database reset completed. Tables and indexes were preserved.");
    Ok(())
}
