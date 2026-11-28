use christmas_greetings::store::Store;
use sha2::{Digest, Sha256};
use std::{env, time::Duration};
use tokio::sync::{broadcast, watch};

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[tokio::test]
async fn changefeed_only_publishes_submitted_greetings_across_campaigns() {
    if env::var_os("RETHINKDB_TESTS").is_none() {
        eprintln!("skipping RethinkDB integration test; set RETHINKDB_TESTS=1 to enable");
        return;
    }

    // Use a dedicated database so this test can run alongside the atomic-claim test.
    env::set_var("RETHINKDB_DB", "congratis_feed_ci");
    let store = Store::connect()
        .await
        .expect("connect and initialize RethinkDB");

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

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
        .to_string();
    let campaign_id = format!("changefeed-{suffix}");
    let token = format!("changefeed-token-{suffix}");
    let hash = token_hash(&token);

    store
        .create_campaign(&campaign_id, "Changefeed test")
        .await
        .expect("create a second campaign");
    store
        .insert_campaign_invitations(&campaign_id, &[(1, hash.clone())])
        .await
        .expect("insert an unused invitation");

    // Creating an unused invitation must not generate a greeting notification.
    let unexpected_campaign_event = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            let event = events_rx.recv().await.expect("broadcast event should be available");
            if event.campaign_id == campaign_id {
                break event;
            }
            // Existing greetings from other campaigns are valid on the shared admin feed.
        }
    })
    .await;
    assert!(
        unexpected_campaign_event.is_err(),
        "unused invitations must be filtered out by the database changefeed"
    );

    assert!(
        store
            .claim_greeting(
                &campaign_id,
                1,
                "Realtime",
                "Tester",
                None,
                "Changefeed delivery works",
            )
            .await
            .expect("claim greeting"),
        "the campaign invitation should be claimed"
    );

    let event = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events_rx.recv().await.expect("broadcast event should be available");
            if event.campaign_id == campaign_id {
                break event;
            }
        }
    })
    .await
    .expect("Changefeed should publish the committed greeting for the target campaign");

    assert_eq!(event.ornament_number, 1);
    assert_eq!(event.display_name, "Realtime Tester");
    assert_eq!(event.message, "Changefeed delivery works");

    feed_task.abort();
}
