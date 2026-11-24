use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use std::{env, io::{self, Write}};

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let count: i16 = env::args().nth(1).unwrap_or_else(|| "20".into()).parse()?;
    anyhow::ensure!((1..=20).contains(&count), "count must be between 1 and 20");
    let base = env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let db = env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&db).await?;

    // Preflight ensures we never partially issue a batch because ornament numbers already exist.
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM participants WHERE ornament_number BETWEEN 1 AND $1")
        .bind(count).fetch_one(&pool).await?;
    anyhow::ensure!(existing == 0, "some requested ornament numbers already have invitations");

    let mut tx = pool.begin().await?;
    let mut rows = Vec::new();
    for number in 1..=count {
        let token = random_token();
        sqlx::query("INSERT INTO participants (ornament_number, token_hash) VALUES ($1, $2)")
            .bind(number).bind(hash(&token)).execute(&mut *tx).await?;
        rows.push((number, token));
    }
    tx.commit().await?;

    println!("ornament_number,personal_url,token");
    for (number, token) in rows {
        println!("{number},{}/?token={},{}", base.trim_end_matches('/'), token, token);
    }
    io::stdout().flush()?;
    eprintln!("Created {count} one-time invitation links. Store this output securely; raw tokens cannot be recovered from the database.");
    Ok(())
}
