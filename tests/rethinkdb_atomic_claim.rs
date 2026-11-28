use christmas_greetings::store::Store;
use sha2::{Digest, Sha256};
use std::env;

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[tokio::test]
async fn concurrent_redemption_claims_invitation_exactly_once() {
    if env::var_os("RETHINKDB_TESTS").is_none() {
        eprintln!("skipping RethinkDB integration test; set RETHINKDB_TESTS=1 to enable");
        return;
    }

    let store = Store::connect().await.expect("connect and initialize RethinkDB");
    let token = format!("integration-{}", uuidless_nonce());
    let hash = token_hash(&token);

    // The CI job uses a dedicated, disposable database for this test.
    let mut invitations = vec![(1, hash.clone())];
    for number in 2..=35 {
        invitations.push((number, token_hash(&format!("{token}-{number}"))));
    }
    assert!(
        store.insert_invitations(&[(36, token_hash("out-of-range"))]).await.is_err(),
        "a campaign must reject more than 35 participants"
    );
    store
        .insert_invitations(&invitations)
        .await
        .expect("seed a 35-participant campaign");
    assert_eq!(
        store.list_participants().await.expect("list campaign participants").len(),
        35
    );

    let (first, second) = tokio::join!(
        store.claim_invitation(&hash, "First", "First concurrent greeting"),
        store.claim_invitation(&hash, "Second", "Second concurrent greeting")
    );

    let first = first.expect("first claim query must complete");
    let second = second.expect("second claim query must complete");
    assert_ne!(
        first.is_some(),
        second.is_some(),
        "exactly one concurrent claim must succeed"
    );

    let greetings = store.list_greetings().await.expect("load committed greeting");
    assert_eq!(greetings.len(), 1);
    assert_eq!(greetings[0].ornament_number, 1);
    assert!(matches!(
        greetings[0].display_name.as_deref(),
        Some("First" | "Second")
    ));
}

fn uuidless_nonce() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}
