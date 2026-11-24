use futures::StreamExt;
use reql::{cmd::connect::Options, r};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host = env::var("RETHINKDB_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = env::var("RETHINKDB_PORT")
        .unwrap_or_else(|_| "28015".into())
        .parse()?;
    let user = env::var("RETHINKDB_USER").unwrap_or_else(|_| "admin".into());
    let password = env::var("RETHINKDB_PASSWORD").unwrap_or_default();

    let options = Options::new()
        .host(host)
        .port(port)
        .user(user)
        .password(password);
    let session = r.connect(options).await?;
    let mut databases = r.db_list().run(&session);
    let mut names = Vec::new();
    while let Some(result) = databases.next().await {
        names.push(result?);
    }
    println!("Connected to RethinkDB; databases: {}", serde_json::to_string(&names)?);
    Ok(())
}
