use christmas_greetings::store::Store;
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::{
    env,
    io::{self, Write},
};

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
    anyhow::ensure!((1..=35).contains(&count), "count must be between 1 and 35");

    let store = Store::connect().await?;

    let mut invitations = Vec::with_capacity(count as usize);
    let mut output = Vec::with_capacity(count as usize);
    for number in 1..=count {
        let token = random_token();
        invitations.push((number, hash(&token)));
        output.push((number, token));
    }

    // RethinkDB has no multi-document transaction. The store performs a
    // preflight empty-table check and inserts the invitation batch once.
    store.insert_invitations(&invitations).await?;

    println!("ornament_number,one_time_token");
    for (number, token) in output {
        println!("{number},{token}");
    }
    io::stdout().flush()?;

    eprintln!(
        "Created {count} one-time invitation links. Store this output securely; raw tokens cannot be recovered from the database."
    );

    Ok(())
}
