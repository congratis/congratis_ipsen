use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Redirect, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use futures::stream;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{convert::Infallible, env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{broadcast, Mutex};
use christmas_greetings::store::{Ornament, ParticipantAccess, RealtimeEvent, Store};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    store: Store,
    events: broadcast::Sender<RealtimeEvent>,
    admin_token: Option<String>,
    admin_mutations: Arc<Mutex<()>>,
}

#[derive(Serialize)]
struct Greeting {
    ornament_number: i16,
    display_name: String,
    message: String,
}

#[derive(Serialize)]
struct AdminOrnament {
    ornament_number: i16,
    used: bool,
    display_name: Option<String>,
    message: Option<String>,
    created_at: i64,
    used_at: Option<i64>,
}

impl From<Ornament> for AdminOrnament {
    fn from(item: Ornament) -> Self {
        Self {
            ornament_number: item.ornament_number,
            used: item.used,
            display_name: item.display_name,
            message: item.message,
            created_at: item.created_at,
            used_at: item.used_at,
        }
    }
}

#[derive(Deserialize)]
struct SubmitGreeting {
    token: String,
    display_name: String,
    message: String,
}

#[derive(Deserialize)]
struct AdminLogin {
    token: String,
}

#[derive(Deserialize)]
struct CreateInvitations {
    count: usize,
}

#[derive(Deserialize)]
struct DeleteOrnament {
    confirm_used: bool,
}

#[derive(Deserialize)]
struct CreateParticipant {
    label: String,
}

#[derive(Serialize)]
struct ParticipantSummary {
    id: String,
    label: String,
    created_at: i64,
    revoked: bool,
}

impl From<ParticipantAccess> for ParticipantSummary {
    fn from(item: ParticipantAccess) -> Self {
        Self {
            id: item.id,
            label: item.label,
            created_at: item.created_at,
            revoked: item.revoked,
        }
    }
}

#[derive(Deserialize)]
struct ParticipantExchange {
    token: String,
}

#[derive(Serialize)]
struct ApiMessage {
    message: &'static str,
}

struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ApiMessage { message: self.1 })).into_response()
    }
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    difference == 0
}

fn authorize_admin(headers: &HeaderMap, configured: &Option<String>) -> Result<(), ApiError> {
    let Some(expected) = configured.as_deref() else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Административный доступ не настроен",
        ));
    };
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();

    if constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Требуется авторизация администратора",
        ))
    }
}

const PARTICIPANT_COOKIE: &str = "congratis_participant";

fn participant_token(headers: &HeaderMap) -> Option<String> {
    headers.get(axum::http::header::COOKIE)?.to_str().ok()?.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == PARTICIPANT_COOKIE).then(|| value.to_string())
    })
}

async fn authorize_participant(headers: &HeaderMap, store: &Store) -> Result<(), ApiError> {
    let token = participant_token(headers).unwrap_or_default();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Нужна персональная ссылка участника"));
    }
    let active = store.participant_is_active(&hash_token(&token)).await.map_err(|error| {
        tracing::error!(?error, "participant access lookup failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось проверить доступ")
    })?;
    if active { Ok(()) } else {
        Err(ApiError(StatusCode::UNAUTHORIZED, "Ссылка участника недействительна или отозвана"))
    }
}

async fn sse_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    authorize_participant(&headers, &state.store).await?;
    let receiver = state.events.subscribe();
    let stream = stream::unfold(receiver, |mut receiver| async move {
        let event = match receiver.recv().await {
            Ok(RealtimeEvent::Greeting(greeting)) => Event::default()
                .event("greeting")
                .json_data(greeting)
                .unwrap_or_else(|_| Event::default().event("resync").data("snapshot")),
            Ok(RealtimeEvent::Removed { ornament_number }) => Event::default()
                .event("ornament_removed")
                .json_data(serde_json::json!({ "ornament_number": ornament_number }))
                .unwrap_or_else(|_| Event::default().event("resync").data("snapshot")),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                Event::default().event("resync").data("snapshot")
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        };
        Some((Ok::<Event, Infallible>(event), receiver))
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    state.store.ping().await.map_err(|error| {
        tracing::error!(?error, "RethinkDB health check failed");
        ApiError(StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
    })?;
    Ok((StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))))
}

async fn list_greetings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Greeting>>, ApiError> {
    authorize_participant(&headers, &state.store).await?;
    let ornaments = state.store.list_greetings().await.map_err(|error| {
        tracing::error!(?error, "failed to load greetings");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Не удалось загрузить поздравления",
        )
    })?;

    Ok(Json(
        ornaments
            .into_iter()
            .filter_map(|ornament| {
                Some(Greeting {
                    ornament_number: ornament.ornament_number,
                    display_name: ornament.display_name?,
                    message: ornament.message?,
                })
            })
            .collect(),
    ))
}

async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SubmitGreeting>,
) -> Result<impl IntoResponse, ApiError> {
    authorize_participant(&headers, &state.store).await?;
    let token = payload.token.trim();
    let name = payload.display_name.trim();
    let message = payload.message.trim();

    if !(64..=128).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректная персональная ссылка"));
    }
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Имя должно содержать от 1 до 80 символов"));
    }
    if message.is_empty() || message.chars().count() > 500 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Поздравление должно содержать от 1 до 500 символов"));
    }

    let ornament_number = state
        .store
        .claim_invitation(&hash_token(token), name, message)
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to claim invitation");
            ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Ошибка базы данных")
        })?
        .ok_or(ApiError(
            StatusCode::CONFLICT,
            "Эта персональная ссылка уже использована или недействительна",
        ))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "message": "Поздравление сохранено",
            "ornament_number": ornament_number
        })),
    ))
}

async fn admin_login(
    State(state): State<AppState>,
    Json(payload): Json<AdminLogin>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(expected) = state.admin_token.as_deref() else {
        return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "Административный доступ не настроен"));
    };
    if !constant_time_eq(payload.token.as_bytes(), expected.as_bytes()) {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Неверный пароль администратора"));
    }
    Ok(Json(serde_json::json!({ "authenticated": true })))
}

async fn participant_exchange(
    State(state): State<AppState>,
    Json(payload): Json<ParticipantExchange>,
) -> Result<impl IntoResponse, ApiError> {
    let token = payload.token.trim();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректная ссылка доступа"));
    }
    if !state.store.participant_is_active(&hash_token(token)).await.map_err(|error| {
        tracing::error!(?error, "participant exchange lookup failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось проверить ссылку")
    })? {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Ссылка участника недействительна или отозвана"));
    }
    let cookie = format!("{PARTICIPANT_COOKIE}={token}; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Lax");
    Ok(([(axum::http::header::SET_COOKIE, cookie)], Json(serde_json::json!({ "authenticated": true }))))
}

async fn participant_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize_participant(&headers, &state.store).await?;
    Ok(Json(serde_json::json!({ "authenticated": true })))
}

async fn participant_logout() -> impl IntoResponse {
    let cookie = format!("{PARTICIPANT_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax");
    ([(axum::http::header::SET_COOKIE, cookie)], Json(serde_json::json!({ "authenticated": false })))
}

async fn admin_list_participants(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ParticipantSummary>>, ApiError> {
    authorize_admin(&headers, &state.admin_token)?;
    let participants = state.store.list_participants().await.map_err(|error| {
        tracing::error!(?error, "admin failed to list participants");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось загрузить участников")
    })?;
    Ok(Json(participants.into_iter().map(ParticipantSummary::from).collect()))
}

async fn admin_create_participant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateParticipant>,
) -> Result<impl IntoResponse, ApiError> {
    authorize_admin(&headers, &state.admin_token)?;
    let label = payload.label.trim();
    if label.is_empty() || label.chars().count() > 80 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Укажите имя участника (до 80 символов)"));
    }
    let mut access_bytes = [0u8; 32];
    let mut id_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut access_bytes);
    OsRng.fill_bytes(&mut id_bytes);
    let token = hex::encode(access_bytes);
    let id = hex::encode(id_bytes);
    let participant = ParticipantAccess {
        id: id.clone(),
        token_hash: hash_token(&token),
        label: label.to_string(),
        created_at: chrono::Utc::now().timestamp(),
        revoked: false,
    };
    state.store.create_participants(&[participant]).await.map_err(|error| {
        tracing::error!(?error, "admin failed to create participant access");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось создать доступ")
    })?;
    let base = env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://congratis.online".into());
    let url = format!("{}/#access={token}", base.trim_end_matches('/'));
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id, "label": label, "url": url }))))
}

async fn admin_revoke_participant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize_admin(&headers, &state.admin_token)?;
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректный идентификатор участника"));
    }
    let revoked = state.store.revoke_participant(&id).await.map_err(|error| {
        tracing::error!(?error, "admin failed to revoke participant");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось отозвать доступ")
    })?;
    if !revoked {
        return Err(ApiError(StatusCode::NOT_FOUND, "Участник не найден"));
    }
    Ok(Json(serde_json::json!({ "revoked": true })))
}

async fn admin_list_ornaments(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminOrnament>>, ApiError> {
    authorize_admin(&headers, &state.admin_token)?;
    let ornaments = state.store.list_ornaments().await.map_err(|error| {
        tracing::error!(?error, "admin failed to list ornaments");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось загрузить приглашения")
    })?;
    Ok(Json(ornaments.into_iter().map(AdminOrnament::from).collect()))
}

async fn admin_create_invitations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateInvitations>,
) -> Result<impl IntoResponse, ApiError> {
    authorize_admin(&headers, &state.admin_token)?;
    let _mutation_guard = state.admin_mutations.lock().await;
    if !(1..=20).contains(&payload.count) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Количество должно быть от 1 до 20"));
    }

    let existing = state.store.list_ornaments().await.map_err(|error| {
        tracing::error!(?error, "admin failed to inspect invitation slots");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось проверить свободные шары")
    })?;
    let occupied: std::collections::HashSet<i16> =
        existing.into_iter().map(|item| item.ornament_number).collect();
    let free_numbers: Vec<i16> = (1..=20).filter(|number| !occupied.contains(number)).collect();
    if payload.count > free_numbers.len() {
        return Err(ApiError(StatusCode::CONFLICT, "Недостаточно свободных шаров"));
    }

    let base = env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://congratis.online".into());
    let mut invitations = Vec::with_capacity(payload.count);
    let mut links = Vec::with_capacity(payload.count);
    for number in free_numbers.into_iter().take(payload.count) {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        invitations.push((number, hash_token(&token)));
        links.push(serde_json::json!({
            "ornament_number": number,
            "url": format!("{}/?token={token}", base.trim_end_matches('/'))
        }));
    }

    state.store.create_invitations(&invitations).await.map_err(|error| {
        tracing::error!(?error, "admin failed to create invitations");
        ApiError(StatusCode::CONFLICT, "Не удалось создать приглашения; обновите список и повторите")
    })?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "created": links }))))
}

async fn admin_delete_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(number): Path<i16>,
    Json(payload): Json<DeleteOrnament>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize_admin(&headers, &state.admin_token)?;
    if !(1..=20).contains(&number) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Номер шара должен быть от 1 до 20"));
    }
    let _mutation_guard = state.admin_mutations.lock().await;
    let deleted = state.store.delete_ornament(number, payload.confirm_used).await.map_err(|error| {
        tracing::error!(?error, ornament_number = number, "admin failed to delete invitation");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось удалить приглашение")
    })?;
    if !deleted {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Приглашение не найдено или уже изменилось; обновите список",
        ));
    }
    Ok(Json(serde_json::json!({ "deleted": true, "ornament_number": number })))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let admin_token = env::var("ADMIN_TOKEN").ok().filter(|token| !token.is_empty());
    if let Some(token) = admin_token.as_ref() {
        anyhow::ensure!(token.len() >= 32, "ADMIN_TOKEN must contain at least 32 characters");
    } else {
        tracing::warn!("ADMIN_TOKEN is not configured; admin API is disabled");
    }

    let store = Store::connect().await?;
    let (events, _) = broadcast::channel(256);
    let feed_store = store.clone();
    let feed_events = events.clone();
    tokio::spawn(async move {
        feed_store.supervise_changefeed(feed_events).await;
    });

    let state = AppState {
        store,
        events,
        admin_token,
        admin_mutations: Arc::new(Mutex::new(())),
    };
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/events", get(sse_events))
        .route("/api/greetings", get(list_greetings).post(submit))
        .route("/api/participant/exchange", post(participant_exchange))
        .route("/api/participant/session", get(participant_session))
        .route("/api/participant/logout", post(participant_logout))
        .route("/api/admin/login", post(admin_login))
        .route("/api/admin/participants", get(admin_list_participants).post(admin_create_participant))
        .route("/api/admin/participants/{id}", delete(admin_revoke_participant))
        .route("/api/admin/ornaments", get(admin_list_ornaments).post(admin_create_invitations))
        .route("/api/admin/ornaments/{number}", delete(admin_delete_invitation))
        .route("/admin", get(|| async { Redirect::permanent("/admin/") }))
        .fallback_service(ServeDir::new("frontend").append_index_html_on_directories(true))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env::var("PORT").unwrap_or_else(|_| "8080".into()).parse()?;
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Congratis listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, hash_token};

    #[test]
    fn token_hash_is_stable_and_hex_encoded() {
        let token = "a".repeat(64);
        let first = hash_token(&token);
        let second = hash_token(&token);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn admin_token_comparison_checks_content_and_length() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret-longer"));
    }
}
