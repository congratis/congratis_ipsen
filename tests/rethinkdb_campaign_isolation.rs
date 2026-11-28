use christmas_greetings::store::{AuthSession, Store};
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}

fn session(id: String, role: &str, campaign_id: Option<&str>, now: i64) -> AuthSession {
    AuthSession {
        id,
        subject_id: "1".to_owned(),
        role: role.to_owned(),
        csrf_hash: "integration-csrf-hash".to_owned(),
        created_at: now,
        expires_at: now + 3600,
        revoked: false,
        campaign_id: campaign_id.map(str::to_owned),
    }
}

#[tokio::test]
async fn campaign_data_and_deletion_are_isolated_and_retryable() {
    if env::var_os("RETHINKDB_TESTS").is_none() {
        eprintln!("skipping RethinkDB integration test; set RETHINKDB_TESTS=1 to enable");
        return;
    }

    let store = Store::connect().await.expect("connect and initialize RethinkDB");
    let suffix = unique_suffix();
    let campaign_a = format!("integration-a-{suffix}");
    let campaign_b = format!("integration-b-{suffix}");
    let name_a = format!("Integration campaign A {suffix}");
    let name_b = format!("Integration campaign B {suffix}");
    let token_a = format!("token-a-{suffix}");
    let token_b = format!("token-b-{suffix}");

    store.create_campaign(&campaign_a, &name_a).await.expect("create campaign A");
    store.create_campaign(&campaign_b, &name_b).await.expect("create campaign B");
    store.insert_campaign_invitations(&campaign_a, &[(1, token_a.clone())]).await
        .expect("insert campaign A invitation");
    store.insert_campaign_invitations(&campaign_b, &[(1, token_b.clone())]).await
        .expect("insert campaign B invitation");

    assert_eq!(store.list_campaign_ornaments(&campaign_a).await.unwrap().len(), 1);
    assert_eq!(store.list_campaign_ornaments(&campaign_b).await.unwrap().len(), 1);

    assert_eq!(
        store.activate_invitation(&token_a).await.unwrap(),
        Some((1, campaign_a.clone()))
    );
    assert_eq!(
        store.activate_invitation(&token_b).await.unwrap(),
        Some((1, campaign_b.clone()))
    );
    assert!(store.claim_greeting(&campaign_a, 1, "Smith", "Alice", None, "Greeting A").await.unwrap());
    assert!(store.claim_greeting(&campaign_b, 1, "Jones", "Bob", None, "Greeting B").await.unwrap());

    let greetings_a = store.list_campaign_greetings(&campaign_a).await.unwrap();
    let greetings_b = store.list_campaign_greetings(&campaign_b).await.unwrap();
    assert_eq!(greetings_a.len(), 1);
    assert_eq!(greetings_a[0].display_name.as_deref(), Some("Smith Alice"));
    assert_eq!(greetings_b.len(), 1);
    assert_eq!(greetings_b[0].display_name.as_deref(), Some("Jones Bob"));

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let session_a = format!("session-a-{suffix}");
    let session_b = format!("session-b-{suffix}");
    let admin_session = format!("session-admin-{suffix}");
    store.create_auth_session(&session(session_a.clone(), "participant", Some(&campaign_a), now)).await
        .expect("create campaign A participant session");
    store.create_auth_session(&session(session_b.clone(), "participant", Some(&campaign_b), now)).await
        .expect("create campaign B participant session");
    store.create_auth_session(&session(admin_session.clone(), "admin", None, now)).await
        .expect("create administrator session");

    assert!(store.delete_campaign(&campaign_a, &name_a).await.unwrap());
    assert!(!store.delete_campaign(&campaign_a, &name_a).await.unwrap(), "deleting an already deleted campaign is a safe no-op");

    assert!(store.get_campaign(&campaign_a).await.unwrap().is_none());
    assert!(store.list_campaign_ornaments(&campaign_a).await.unwrap().is_empty());
    assert!(store.list_campaign_greetings(&campaign_a).await.unwrap().is_empty());
    assert!(store.get_auth_session(&session_a).await.unwrap().is_none());

    assert_eq!(store.get_campaign(&campaign_b).await.unwrap().unwrap().name, name_b);
    let remaining_b = store.list_campaign_greetings(&campaign_b).await.unwrap();
    assert_eq!(remaining_b.len(), 1);
    assert_eq!(remaining_b[0].display_name.as_deref(), Some("Bob"));
    assert!(store.get_auth_session(&session_b).await.unwrap().is_some());
    assert!(store.get_auth_session(&admin_session).await.unwrap().is_some());

    assert!(!store.claim_greeting(&campaign_a, 1, "Evil", "Mallory", None, "Must not be stored").await.unwrap());
    assert_eq!(store.activate_invitation(&token_a).await.unwrap(), None);

    let legacy = match store.get_campaign("legacy-default").await.unwrap() {
        Some(campaign) => campaign,
        None => store.create_campaign("legacy-default", "Существующая кампания").await.unwrap(),
    };
    assert!(
        store.delete_campaign("legacy-default", &legacy.name).await.is_err(),
        "the legacy campaign must remain protected"
    );
}
