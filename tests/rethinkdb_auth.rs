use christmas_greetings::store::{AuthSession, Store};
use sha2::{Digest, Sha256};
use std::env;

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[tokio::test]
async fn invitation_activates_once_and_session_can_be_revoked() {
    if env::var_os("RETHINKDB_TESTS").is_none() {
        eprintln!("skipping RethinkDB auth test; set RETHINKDB_TESTS=1 to enable");
        return;
    }
    env::set_var("RETHINKDB_DB", "congratis_auth_ci");
    let store = Store::connect().await.expect("connect to auth test database");
    let token = format!("auth-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let token_hash = hash(&token);
    store.insert_invitations(&[(1, token_hash.clone())]).await.expect("seed invitation");

    assert_eq!(store.activate_invitation(&token_hash).await.unwrap(), Some((1, "legacy-default".to_owned())));
    assert_eq!(store.activate_invitation(&token_hash).await.unwrap(), None);

    let session = AuthSession {
        id: hash("session-secret"),
        subject_id: "1".into(),
        role: "participant".into(),
        csrf_hash: hash("csrf-secret"),
        created_at: 1_800_000_000,
        expires_at: 1_900_000_000,
        revoked: false,
        campaign_id: Some("legacy-default".into()),
    };
    store.create_auth_session(&session).await.expect("create session");
    let loaded = store.get_auth_session(&session.id).await.unwrap().expect("load session");
    assert_eq!(loaded.subject_id, "1");
    assert_eq!(loaded.role, "participant");
    store.revoke_auth_session(&session.id).await.expect("revoke session");
    assert!(store.get_auth_session(&session.id).await.unwrap().unwrap().revoked);

    assert!(store.claim_greeting("legacy-default", 1, "Tester", "First", None, "One greeting only").await.unwrap());
    assert!(!store.claim_greeting("legacy-default", 1, "Tester", "Second", None, "Replay").await.unwrap());
}
