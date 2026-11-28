use christmas_greetings::store::Store;
use sha2::{Digest, Sha256};
use std::{env, time::Duration};
use tokio::sync::{broadcast, watch};

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[tokio::test]
async fn committed_greeting_is_published_by_changefeed() {
    if env::var_os("RETHINKDB_TESTS").is_none() {
        eprintln!("skipping RethinkDB integration test; set RETHINKDB_TESTS=1 to enable");
        return;
    }

    // Use a dedicated database so this test can run alongside the atomic-claim test.
    env::set_var("RETHINKDB_DB", "congratis_feed_ci");
    let store = Store::connect()
        .await
        .expect("connect and initialize RethinkDB");

    let token = format!(
        "changefeed-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    );
    let hash = token_hash(&token);
    store
        .insert_invitations(&[(1, hash.clone())])
        .await
        .expect("seed one invitation");

    let (events_tx, mut events_rx) = broadcast::channel(16);
    let (ready_tx, mut ready_rx) = watch::channel(false);
    let feed_store = store.clone();
    let feed_task = tokio::spawn(async move {
        let _ = feed_store.subscribe_changefeed(&events_tx, ready_tx).await;
    });

    tokio::time::timeout(Duration::from_secs(10), async {
        while !*ready_rx.borrow() {
            ready_rx
                .changed()
                .await
                .expect("Changefeed readiness channel");
        }
    })
    .await
    .expect("Changefeed must finish its initial snapshot");

    let ornament = store
        .claim_invitation(&hash, "Realtime tester", "Changefeed delivery works")
        .await
        .expect("claim invitation")
        .expect("invitation should be available");
    assert_eq!(ornament, 1);

    let event = tokio::time::timeout(Duration::from_secs(10), events_rx.recv())
        .await
        .expect("Changefeed should publish the committed greeting")
        .expect("broadcast event should be available");

    assert_eq!(event.ornament_number, 1);
    assert_eq!(event.display_name, "Realtime tester");
    assert_eq!(event.message, "Changefeed delivery works");

    feed_task.abort();
}
