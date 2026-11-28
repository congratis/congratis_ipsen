use argon2::{
    password_hash::{PasswordHash, PasswordVerifier},
    Argon2,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use christmas_greetings::store::{AuthSession, GreetingEvent, Store};
use futures::{stream, SinkExt};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

const SESSION_COOKIE: &str = "congratis_session";
const CSRF_COOKIE: &str = "congratis_csrf";
const PARTICIPANT_TTL: i64 = 60 * 60 * 24 * 30;
const ADMIN_TTL: i64 = 60 * 60 * 8;

#[derive(Clone)]
struct AppState {
    store: Store,
    events: broadcast::Sender<GreetingEvent>,
    presence_events: broadcast::Sender<PresenceUpdate>,
    presence: Arc<Mutex<HashMap<(String, i16), PresenceRecord>>>,
    cookie_secure: bool,
}

#[derive(Serialize)]
struct Greeting {
    ornament_number: i16,
    display_name: String,
    surname: String,
    first_name: String,
    avatar_url: Option<String>,
    message: String,
    like_count: usize,
    liked_by_me: bool,
}
#[derive(Serialize)]
struct LeaderboardEntry {
    display_name: String,
    avatar_url: Option<String>,
    like_count: usize,
}
#[derive(Serialize)]
struct CampaignSummary {
    participant_count: usize,
    submitted_count: usize,
    campaign_name: String,
}
#[derive(Deserialize)]
struct ParticipantExchange {
    token: String,
}
#[derive(Deserialize)]
struct AdminLogin {
    username: String,
    password: String,
}
#[derive(Deserialize)]
struct CreateCampaign {
    participant_count: i16,
}
#[derive(Deserialize)]
struct CreateNamedCampaign {
    name: String,
    participant_count: i16,
}
#[derive(Deserialize)]
struct AddCampaignInvitations {
    count: i16,
}
#[derive(Deserialize)]
struct DeleteCampaign {
    confirm_name: String,
}
#[derive(Serialize)]
struct CampaignAdminItem {
    id: String,
    name: String,
    created_at: i64,
    participant_count: usize,
    submitted_count: usize,
}
#[derive(Serialize)]
struct CreateCampaignResponse {
    campaign: CampaignAdminItem,
    invitations: Vec<CreatedInvitation>,
}
#[derive(Serialize)]
struct CreatedInvitation {
    ornament_number: i16,
    token: String,
}
#[derive(Deserialize)]
struct SubmitGreeting {
    surname: String,
    first_name: String,
    message: String,
    #[serde(default)]
    avatar_data_url: Option<String>,
}
#[derive(Serialize)]
struct ApiMessage {
    message: &'static str,
}
#[derive(Serialize)]
struct MeResponse {
    subject_id: String,
    role: String,
    csrf_token: String,
}
#[derive(Debug)]
struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ApiMessage { message: self.1 })).into_response()
    }
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn campaign_numbers(count: i16) -> Result<Vec<i16>, ApiError> {
    if !(1..=35).contains(&count) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Количество участников должно быть от 1 до 35",
        ));
    }
    Ok((1..=count).collect())
}

fn add_campaign_count(
    counts: &mut HashMap<String, (usize, usize)>,
    campaign_id: Option<&str>,
    submitted: bool,
) {
    let campaign_id = campaign_id
        .unwrap_or(christmas_greetings::store::LEGACY_CAMPAIGN_ID)
        .to_owned();
    let entry = counts.entry(campaign_id).or_default();
    entry.0 += 1;
    if submitted {
        entry.1 += 1;
    }
}
fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn random_campaign_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
}
fn secure_suffix(state: &AppState) -> &'static str {
    if state.cookie_secure {
        "; Secure"
    } else {
        ""
    }
}
fn issue_cookies(response: &mut Response, state: &AppState, session: &str, csrf: &str, ttl: i64) {
    let headers = response.headers_mut();
    let session_cookie = format!(
        "{SESSION_COOKIE}={session}; Path=/; Max-Age={ttl}; HttpOnly; SameSite=Strict{}",
        secure_suffix(state)
    );
    let csrf_cookie = format!(
        "{CSRF_COOKIE}={csrf}; Path=/; Max-Age={ttl}; SameSite=Strict{}",
        secure_suffix(state)
    );
    if let Ok(v) = HeaderValue::from_str(&session_cookie) {
        headers.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&csrf_cookie) {
        headers.append(header::SET_COOKIE, v);
    }
}
fn clear_cookies(response: &mut Response, state: &AppState) {
    for value in [
        format!(
            "{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{}",
            secure_suffix(state)
        ),
        format!(
            "{CSRF_COOKIE}=; Path=/; Max-Age=0; SameSite=Strict{}",
            secure_suffix(state)
        ),
    ] {
        if let Ok(v) = HeaderValue::from_str(&value) {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
    }
}
async fn authenticated(state: &AppState, headers: &HeaderMap) -> Result<AuthSession, ApiError> {
    let raw = cookie_value(headers, SESSION_COOKIE)
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "Требуется авторизация"))?;
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Недействительная сессия",
        ));
    }
    let session = state
        .store
        .get_auth_session(&digest(&raw))
        .await
        .map_err(|e| {
            tracing::error!(?e, "session lookup failed");
            ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Ошибка проверки сессии")
        })?
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "Сессия не найдена"))?;
    if session.revoked {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Сессия отозвана"));
    }
    if session.expires_at <= chrono::Utc::now().timestamp() {
        let _ = state.store.revoke_auth_session(&session.id).await;
        return Err(ApiError(StatusCode::UNAUTHORIZED, "Сессия истекла"));
    }
    Ok(session)
}
fn verify_csrf(headers: &HeaderMap, session: &AuthSession) -> Result<(), ApiError> {
    let cookie = cookie_value(headers, CSRF_COOKIE)
        .ok_or(ApiError(StatusCode::FORBIDDEN, "Отсутствует CSRF cookie"))?;
    let supplied = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError(StatusCode::FORBIDDEN, "Отсутствует CSRF token"))?;
    if cookie != supplied || digest(supplied) != session.csrf_hash {
        return Err(ApiError(StatusCode::FORBIDDEN, "Некорректный CSRF token"));
    }
    Ok(())
}
async fn new_session(
    state: &AppState,
    subject_id: String,
    role: &str,
    campaign_id: Option<String>,
    ttl: i64,
) -> Result<(String, String), ApiError> {
    let raw_session = random_secret();
    let raw_csrf = random_secret();
    let now = chrono::Utc::now().timestamp();
    let session = AuthSession {
        id: digest(&raw_session),
        subject_id,
        role: role.to_owned(),
        csrf_hash: digest(&raw_csrf),
        created_at: now,
        expires_at: now + ttl,
        revoked: false,
        campaign_id,
    };
    state
        .store
        .create_auth_session(&session)
        .await
        .map_err(|e| {
            tracing::error!(?e, "session creation failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось создать сессию",
            )
        })?;
    Ok((raw_session, raw_csrf))
}

async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    state.store.ping().await.map_err(|e| {
        tracing::error!(?e, "database health check failed");
        ApiError(StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
    })?;
    Ok((StatusCode::OK, Json(serde_json::json!({"status":"ok"}))))
}
async fn participant_exchange(
    State(state): State<AppState>,
    Json(payload): Json<ParticipantExchange>,
) -> Result<Response, ApiError> {
    let token = payload.token.trim();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Некорректное приглашение",
        ));
    }
    let (ornament, campaign_id) = state
        .store
        .activate_invitation(&digest(token))
        .await
        .map_err(|e| {
            tracing::error!(?e, "invitation activation failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Ошибка активации приглашения",
            )
        })?
        .ok_or(ApiError(
            StatusCode::CONFLICT,
            "Приглашение недействительно или уже использовано",
        ))?;
    let (session, csrf) = match new_session(
        &state,
        ornament.to_string(),
        "participant",
        Some(campaign_id.clone()),
        PARTICIPANT_TTL,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            if let Err(rollback_error) = state
                .store
                .rollback_invitation_activation(&digest(token))
                .await
            {
                tracing::error!(
                    ?rollback_error,
                    "failed to roll back invitation activation after session creation error"
                );
            }
            return Err(error);
        }
    };
    let mut response = (
        StatusCode::OK,
        Json(MeResponse {
            subject_id: ornament.to_string(),
            role: "participant".into(),
            csrf_token: csrf.clone(),
        }),
    )
        .into_response();
    issue_cookies(&mut response, &state, &session, &csrf, PARTICIPANT_TTL);
    Ok(response)
}
async fn admin_login(
    State(state): State<AppState>,
    Json(payload): Json<AdminLogin>,
) -> Result<Response, ApiError> {
    let username = env::var("ADMIN_USERNAME").unwrap_or_default();
    let encoded_hash = env::var("ADMIN_PASSWORD_HASH").unwrap_or_default();
    if username.is_empty() || encoded_hash.is_empty() || payload.username != username {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Неверный логин или пароль",
        ));
    }
    let parsed = PasswordHash::new(&encoded_hash).map_err(|_| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Некорректная конфигурация пароля администратора",
        )
    })?;
    if Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed)
        .is_err()
    {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "Неверный логин или пароль",
        ));
    }
    let (session, csrf) = new_session(&state, username, "admin", None, ADMIN_TTL).await?;
    let mut response = (
        StatusCode::OK,
        Json(MeResponse {
            subject_id: "admin".into(),
            role: "admin".into(),
            csrf_token: csrf.clone(),
        }),
    )
        .into_response();
    issue_cookies(&mut response, &state, &session, &csrf, ADMIN_TTL);
    Ok(response)
}
async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    let csrf = cookie_value(&headers, CSRF_COOKIE).unwrap_or_default();
    Ok(Json(MeResponse {
        subject_id: session.subject_id,
        role: session.role,
        csrf_token: csrf,
    }))
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let session = authenticated(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    state
        .store
        .revoke_auth_session(&session.id)
        .await
        .map_err(|e| {
            tracing::error!(?e, "session revocation failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось завершить сессию",
            )
        })?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    clear_cookies(&mut response, &state);
    Ok(response)
}
async fn campaign_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CampaignSummary>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    let campaign_id = session
        .campaign_id
        .as_deref()
        .unwrap_or(christmas_greetings::store::LEGACY_CAMPAIGN_ID);
    let participants = state
        .store
        .list_campaign_participants(campaign_id)
        .await
        .map_err(|e| {
            tracing::error!(?e, "campaign summary lookup failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось загрузить состояние кампании",
            )
        })?;
    let participant_count = participants.len();
    let submitted_count = participants
        .iter()
        .filter(|participant| participant.submitted)
        .count();
    let campaign_name = state
        .store
        .get_campaign(campaign_id)
        .await
        .map_err(|e| {
            tracing::error!(?e, "campaign title lookup failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось загрузить название кампании",
            )
        })?
        .map(|campaign| campaign.name)
        .unwrap_or_else(|| "Congratis".to_owned());
    Ok(Json(CampaignSummary {
        participant_count,
        submitted_count,
        campaign_name,
    }))
}

async fn list_greetings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Greeting>>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    let campaign_id = session
        .campaign_id
        .as_deref()
        .unwrap_or(christmas_greetings::store::LEGACY_CAMPAIGN_ID);
    let ornaments = state
        .store
        .list_campaign_greetings(campaign_id)
        .await
        .map_err(|e| {
            tracing::error!(?e, "failed to load greetings");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось загрузить поздравления",
            )
        })?;
    Ok(Json(
        ornaments
            .into_iter()
            .filter_map(|o| {
                Some(Greeting {
                    ornament_number: o.ornament_number,
                    display_name: o.display_name?,
                    surname: o.surname.unwrap_or_default(),
                    first_name: o.first_name.unwrap_or_default(),
                    avatar_url: o.avatar_filename.map(|name| format!("/avatars/{name}")),
                    message: o.message?,
                    like_count: o.liked_by.len(),
                    liked_by_me: session.role == "participant"
                        && session
                            .subject_id
                            .parse::<i16>()
                            .ok()
                            .is_some_and(|number| o.liked_by.contains(&number)),
                })
            })
            .collect(),
    ))
}

async fn toggle_greeting_like(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(target_number): axum::extract::Path<i16>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "participant" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Только участник может ставить лайки",
        ));
    }
    verify_csrf(&headers, &session)?;
    let voter_number = session.subject_id.parse::<i16>().map_err(|_| {
        ApiError(
            StatusCode::FORBIDDEN,
            "Некорректная учётная запись участника",
        )
    })?;
    if !(1..=35).contains(&target_number) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Некорректный номер поздравления",
        ));
    }
    let campaign_id = session
        .campaign_id
        .as_deref()
        .unwrap_or(christmas_greetings::store::LEGACY_CAMPAIGN_ID);
    let Some((liked, like_count)) = state.store.toggle_greeting_like(campaign_id, voter_number, target_number).await.map_err(|error| {
        tracing::error!(?error, campaign_id = %campaign_id, target_number, "greeting like toggle failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось сохранить отметку «Нравится»")
    })? else {
        return Err(ApiError(StatusCode::CONFLICT, "Нельзя отметить собственное или неопубликованное поздравление"));
    };
    Ok(Json(
        serde_json::json!({"liked": liked, "like_count": like_count}),
    ))
}

async fn leaderboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<LeaderboardEntry>>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    let campaign_id = session
        .campaign_id
        .as_deref()
        .unwrap_or(christmas_greetings::store::LEGACY_CAMPAIGN_ID);
    let mut entries: Vec<LeaderboardEntry> = state
        .store
        .list_campaign_greetings(campaign_id)
        .await
        .map_err(|error| {
            tracing::error!(?error, "leaderboard lookup failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось загрузить рейтинг поздравлений",
            )
        })?
        .into_iter()
        .filter_map(|item| {
            Some(LeaderboardEntry {
                display_name: item.display_name?,
                avatar_url: item.avatar_filename.map(|name| format!("/avatars/{name}")),
                like_count: item.liked_by.len(),
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.like_count
            .cmp(&a.like_count)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    entries.truncate(5);
    Ok(Json(entries))
}

async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SubmitGreeting>,
) -> Result<impl IntoResponse, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "participant" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Только участник может отправить поздравление",
        ));
    }
    verify_csrf(&headers, &session)?;
    let ornament: i16 = session.subject_id.parse().map_err(|_| {
        ApiError(
            StatusCode::FORBIDDEN,
            "Некорректная учётная запись участника",
        )
    })?;
    let surname = payload.surname.trim();
    let first_name = payload.first_name.trim();
    let message = payload.message.trim();
    if surname.is_empty()
        || surname.chars().count() > 80
        || first_name.is_empty()
        || first_name.chars().count() > 80
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Фамилия и имя обязательны (до 80 символов каждое)",
        ));
    }
    if message.is_empty() || message.chars().count() > 500 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Поздравление должно содержать от 1 до 500 символов",
        ));
    }
    let mut avatar_filename: Option<String> = None;
    if let Some(data_url) = payload.avatar_data_url.as_deref() {
        let (declared_type, encoded) = data_url.split_once(",").ok_or(ApiError(
            StatusCode::BAD_REQUEST,
            "Некорректный формат фотографии",
        ))?;
        if !matches!(
            declared_type,
            "data:image/jpeg;base64" | "data:image/png;base64" | "data:image/webp;base64"
        ) {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "Поддерживаются только JPEG, PNG и WebP",
            ));
        }
        let bytes = BASE64
            .decode(encoded)
            .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Некорректные данные фотографии"))?;
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Фотография должна быть не больше 1 МБ",
            ));
        }
        let format = image::guess_format(&bytes).map_err(|_| {
            ApiError(
                StatusCode::BAD_REQUEST,
                "Файл не является корректным изображением",
            )
        })?;
        let expected = match declared_type {
            "data:image/jpeg;base64" => image::ImageFormat::Jpeg,
            "data:image/png;base64" => image::ImageFormat::Png,
            _ => image::ImageFormat::WebP,
        };
        if format != expected {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "Тип файла не совпадает с содержимым",
            ));
        }
        let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
            .into_dimensions()
            .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Не удалось прочитать изображение"))?;
        if width == 0
            || height == 0
            || width > 10_000
            || height > 10_000
            || (width as u64) * (height as u64) > 40_000_000
        {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "Слишком большое разрешение фотографии",
            ));
        }
        let decoded = image::load_from_memory_with_format(&bytes, format)
            .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "Не удалось прочитать изображение"))?;
        let normalized = decoded.thumbnail(512, 512);
        let mut output = std::io::Cursor::new(Vec::new());
        normalized
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|_| {
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Не удалось обработать фотографию",
                )
            })?;
        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
        let filename = format!("{}.png", hex::encode(random));
        let directory = env::var("AVATAR_DIR").unwrap_or_else(|_| "/data/avatars".to_owned());
        tokio::fs::create_dir_all(&directory).await.map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Хранилище фотографий недоступно",
            )
        })?;
        let path = std::path::Path::new(&directory).join(&filename);
        tokio::fs::write(&path, output.into_inner())
            .await
            .map_err(|_| {
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Не удалось сохранить фотографию",
                )
            })?;
        avatar_filename = Some(filename);
    }
    let campaign_id = session
        .campaign_id
        .as_deref()
        .unwrap_or(christmas_greetings::store::LEGACY_CAMPAIGN_ID);
    let claimed = state
        .store
        .claim_greeting(
            campaign_id,
            ornament,
            surname,
            first_name,
            avatar_filename.as_deref(),
            message,
        )
        .await
        .map_err(|e| {
            tracing::error!(?e, "greeting claim failed");
            ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Ошибка базы данных")
        })?;
    if !claimed {
        if let Some(filename) = avatar_filename.as_deref() {
            let directory = env::var("AVATAR_DIR").unwrap_or_else(|_| "/data/avatars".to_owned());
            let _ = tokio::fs::remove_file(std::path::Path::new(&directory).join(filename)).await;
        }
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Поздравление уже отправлено",
        ));
    }
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "message":"Поздравление сохранено", "ornament_number":ornament
        })),
    ))
}
async fn admin_create_campaign(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateCampaign>,
) -> Result<(StatusCode, Json<Vec<CreatedInvitation>>), ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;

    let numbers = campaign_numbers(payload.participant_count)?;
    let mut invitations = Vec::with_capacity(numbers.len());
    let mut created = Vec::with_capacity(numbers.len());
    for number in numbers {
        let token = random_secret();
        invitations.push((number, digest(&token)));
        created.push(CreatedInvitation {
            ornament_number: number,
            token,
        });
    }
    state
        .store
        .insert_invitations(&invitations)
        .await
        .map_err(|error| {
            if error.to_string().contains("not empty") {
                ApiError(
                    StatusCode::CONFLICT,
                    "Кампания уже создана; изменить занятые слоты нельзя",
                )
            } else {
                tracing::error!(?error, "campaign creation failed");
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Не удалось создать кампанию",
                )
            }
        })?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn admin_participants(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<christmas_greetings::store::ParticipantSummary>>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    let participants = state.store.list_participants().await.map_err(|e| {
        tracing::error!(?e, "participant listing failed");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Не удалось загрузить участников",
        )
    })?;
    Ok(Json(participants))
}
async fn admin_reissue_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(number): axum::extract::Path<i16>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;
    if !(1..=35).contains(&number) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректный номер шара"));
    }
    let token = random_secret();
    let changed = state
        .store
        .reissue_invitation(number, &digest(&token))
        .await
        .map_err(|e| {
            tracing::error!(?e, "invitation reissue failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось перевыпустить приглашение",
            )
        })?;
    if !changed {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Приглашение уже активировано, использовано или номер не существует",
        ));
    }
    Ok(Json(
        serde_json::json!({"ornament_number":number,"token":token}),
    ))
}
async fn admin_revoke_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(number): axum::extract::Path<i16>,
) -> Result<StatusCode, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;
    if !(1..=35).contains(&number) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректный номер шара"));
    }
    let changed = state.store.revoke_invitation(number).await.map_err(|e| {
        tracing::error!(?e, "invitation revoke failed");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Не удалось отозвать приглашение",
        )
    })?;
    if !changed {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Приглашение уже активировано, использовано или номер не существует",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_list_campaigns(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CampaignAdminItem>>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    let campaigns = state.store.list_campaigns().await.map_err(|error| {
        tracing::error!(?error, "campaign list failed");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Не удалось загрузить кампании",
        )
    })?;
    // One table read for all campaigns replaces the previous per-campaign
    // participant lookup (N+1). Counts only need invitation state, not likes.
    let ornaments = state.store.list_ornaments().await.map_err(|error| {
        tracing::error!(?error, "campaign invitation counts lookup failed");
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Не удалось загрузить данные кампании",
        )
    })?;
    let mut counts = HashMap::<String, (usize, usize)>::new();
    for ornament in ornaments {
        add_campaign_count(&mut counts, ornament.campaign_id.as_deref(), ornament.used);
    }

    let items = campaigns
        .into_iter()
        .map(|campaign| {
            let (participant_count, submitted_count) =
                counts.get(&campaign.id).copied().unwrap_or_default();
            CampaignAdminItem {
                id: campaign.id,
                name: campaign.name,
                created_at: campaign.created_at,
                participant_count,
                submitted_count,
            }
        })
        .collect();
    Ok(Json(items))
}

async fn admin_create_named_campaign(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateNamedCampaign>,
) -> Result<(StatusCode, Json<CreateCampaignResponse>), ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;

    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Название кампании должно содержать от 1 до 100 символов",
        ));
    }
    let numbers = campaign_numbers(payload.participant_count)?;
    let campaign_id = random_campaign_id();
    let campaign = state
        .store
        .create_campaign(&campaign_id, name)
        .await
        .map_err(|error| {
            tracing::error!(?error, "campaign creation failed");
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Не удалось создать кампанию",
            )
        })?;

    let mut hashes = Vec::with_capacity(numbers.len());
    let mut invitations = Vec::with_capacity(numbers.len());
    for number in numbers {
        let token = random_secret();
        hashes.push((number, digest(&token)));
        invitations.push(CreatedInvitation {
            ornament_number: number,
            token,
        });
    }
    if let Err(error) = state
        .store
        .insert_campaign_invitations(&campaign_id, &hashes)
        .await
    {
        tracing::error!(?error, campaign_id = %campaign_id, "campaign invitations creation failed after campaign record creation");
        return Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Кампания создана, но не удалось создать приглашения; повторите операцию после проверки кампании"));
    }

    let campaign_item = CampaignAdminItem {
        id: campaign.id,
        name: campaign.name,
        created_at: campaign.created_at,
        participant_count: invitations.len(),
        submitted_count: 0,
    };
    Ok((
        StatusCode::CREATED,
        Json(CreateCampaignResponse {
            campaign: campaign_item,
            invitations,
        }),
    ))
}

async fn admin_delete_campaign(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(campaign_id): axum::extract::Path<String>,
    Json(payload): Json<DeleteCampaign>,
) -> Result<StatusCode, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;

    let deleted = state
        .store
        .delete_campaign(&campaign_id, &payload.confirm_name)
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("legacy campaign") {
                ApiError(
                    StatusCode::FORBIDDEN,
                    "Существующую кампанию удалить нельзя",
                )
            } else if message.contains("confirmation") {
                ApiError(StatusCode::BAD_REQUEST, "Название кампании не совпадает")
            } else {
                tracing::error!(?error, campaign_id = %campaign_id, "campaign deletion failed");
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Не удалось удалить кампанию",
                )
            }
        })?;
    if !deleted {
        return Err(ApiError(StatusCode::NOT_FOUND, "Кампания не найдена"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_campaign_participants(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(campaign_id): axum::extract::Path<String>,
) -> Result<Json<Vec<christmas_greetings::store::ParticipantSummary>>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    let participants = state.store.list_campaign_participants(&campaign_id).await.map_err(|error| {
        tracing::error!(?error, campaign_id = %campaign_id, "campaign participants lookup failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось загрузить участников кампании")
    })?;
    Ok(Json(participants))
}

async fn admin_add_campaign_invitations(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(campaign_id): axum::extract::Path<String>,
    Json(payload): Json<AddCampaignInvitations>,
) -> Result<(StatusCode, Json<Vec<CreatedInvitation>>), ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;
    let count = payload.count;
    if !(1..=35).contains(&count) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Количество приглашений должно быть от 1 до 35",
        ));
    }
    let existing = state.store.list_campaign_ornaments(&campaign_id).await.map_err(|error| {
        tracing::error!(?error, campaign_id = %campaign_id, "campaign invitation lookup failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось загрузить приглашения кампании")
    })?;
    let occupied: std::collections::HashSet<i16> =
        existing.iter().map(|item| item.ornament_number).collect();
    let available: Vec<i16> = (1..=35)
        .filter(|number| !occupied.contains(number))
        .take(count as usize)
        .collect();
    if available.len() != count as usize {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "В кампании недостаточно свободных номеров шаров",
        ));
    }
    let mut hashes = Vec::with_capacity(available.len());
    let mut invitations = Vec::with_capacity(available.len());
    for number in available {
        let token = random_secret();
        hashes.push((number, digest(&token)));
        invitations.push(CreatedInvitation {
            ornament_number: number,
            token,
        });
    }
    state.store.insert_campaign_invitations(&campaign_id, &hashes).await.map_err(|error| {
        tracing::error!(?error, campaign_id = %campaign_id, "adding campaign invitations failed");
        ApiError(StatusCode::CONFLICT, "Не удалось добавить приглашения; проверьте занятые номера и повторите попытку")
    })?;
    Ok((StatusCode::CREATED, Json(invitations)))
}

async fn admin_reissue_campaign_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path((campaign_id, number)): axum::extract::Path<(String, i16)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;
    if !(1..=35).contains(&number) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректный номер шара"));
    }
    let token = random_secret();
    let changed = state.store.reissue_campaign_invitation(&campaign_id, number, &digest(&token)).await.map_err(|error| {
        tracing::error!(?error, campaign_id = %campaign_id, number, "campaign invitation reissue failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось перевыпустить приглашение")
    })?;
    if !changed {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Приглашение уже активировано, использовано или не существует",
        ));
    }
    Ok(Json(
        serde_json::json!({"ornament_number":number,"token":token}),
    ))
}

async fn admin_revoke_campaign_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path((campaign_id, number)): axum::extract::Path<(String, i16)>,
) -> Result<StatusCode, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    verify_csrf(&headers, &session)?;
    if !(1..=35).contains(&number) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Некорректный номер шара"));
    }
    let changed = state.store.revoke_campaign_invitation(&campaign_id, number).await.map_err(|error| {
        tracing::error!(?error, campaign_id = %campaign_id, number, "campaign invitation revoke failed");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось отозвать приглашение")
    })?;
    if !changed {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Приглашение уже активировано, использовано или не существует",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone)]
struct PresenceRecord {
    connections: usize,
    last_activity: i64,
    filling: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PresenceUpdate {
    kind: String,
    campaign_id: String,
    ornament_number: i16,
    online: bool,
    filling: bool,
    online_count: usize,
}

fn presence_message(update: &PresenceUpdate) -> Result<Message, serde_json::Error> {
    Ok(Message::Text(serde_json::to_string(update)?.into()))
}

async fn publish_presence(state: &AppState, campaign_id: &str, ornament_number: i16) {
    let now = chrono::Utc::now().timestamp();
    let presence = state.presence.lock().await;
    let online_count = presence
        .iter()
        .filter(|((campaign, _), record)| campaign == campaign_id && record.connections > 0)
        .count();
    let record = presence.get(&(campaign_id.to_owned(), ornament_number));
    let online = record.is_some_and(|item| item.connections > 0);
    let filling =
        record.is_some_and(|item| online && item.filling && now - item.last_activity <= 30);
    drop(presence);
    let _ = state.presence_events.send(PresenceUpdate {
        kind: "presence".into(),
        campaign_id: campaign_id.to_owned(),
        ornament_number,
        online,
        filling,
        online_count,
    });
}

async fn presence_connection(mut socket: WebSocket, state: AppState, session: AuthSession) {
    let is_admin = session.role == "admin";
    let campaign_id = session
        .campaign_id
        .clone()
        .unwrap_or_else(|| christmas_greetings::store::LEGACY_CAMPAIGN_ID.to_owned());
    let ornament_number = if is_admin {
        None
    } else {
        session.subject_id.parse::<i16>().ok()
    };
    let mut receiver = state.presence_events.subscribe();

    if let Some(number) = ornament_number {
        {
            let mut presence = state.presence.lock().await;
            let record = presence
                .entry((campaign_id.clone(), number))
                .or_insert(PresenceRecord {
                    connections: 0,
                    last_activity: 0,
                    filling: false,
                });
            record.connections += 1;
        }
        publish_presence(&state, &campaign_id, number).await;
    } else if is_admin {
        let snapshot = state.presence.lock().await.clone();
        for ((campaign, number), record) in snapshot {
            let now = chrono::Utc::now().timestamp();
            let online_count = state
                .presence
                .lock()
                .await
                .iter()
                .filter(|((id, _), value)| id == &campaign && value.connections > 0)
                .count();
            let update = PresenceUpdate {
                kind: "presence".into(),
                campaign_id: campaign,
                ornament_number: number,
                online: record.connections > 0,
                filling: record.connections > 0
                    && record.filling
                    && now - record.last_activity <= 30,
                online_count,
            };
            if let Ok(message) = presence_message(&update) {
                if socket.send(message).await.is_err() {
                    return;
                }
            }
        }
    } else {
        let _ = socket.close().await;
        return;
    }

    let mut ping_interval = tokio::time::interval(Duration::from_secs(20));
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) if !is_admin => {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                            if value.get("type").and_then(|v| v.as_str()) == Some("activity") {
                                if let Some(number) = ornament_number {
                                    if let Some(record) = state.presence.lock().await.get_mut(&(campaign_id.clone(), number)) {
                                        record.last_activity = chrono::Utc::now().timestamp();
                                        record.filling = true;
                                    }
                                    publish_presence(&state, &campaign_id, number).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Pong(_))) => last_pong = Instant::now(),
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = receiver.recv() => {
                if let Ok(update) = event {
                    if is_admin || update.campaign_id == campaign_id {
                        let outgoing = if is_admin {
                            presence_message(&update)
                        } else {
                            Ok(Message::Text(serde_json::json!({
                                "kind": "count",
                                "online_count": update.online_count
                            }).to_string().into()))
                        };
                        if let Ok(message) = outgoing {
                            if socket.send(message).await.is_err() { break; }
                        }
                    }
                }
            }
            _ = ping_interval.tick() => {
                if last_pong.elapsed() > Duration::from_secs(45) { break; }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
                if let Some(number) = ornament_number {
                    let now = chrono::Utc::now().timestamp();
                    let expired = {
                        let mut presence = state.presence.lock().await;
                        if let Some(record) = presence.get_mut(&(campaign_id.clone(), number)) {
                            if record.filling && now - record.last_activity > 30 {
                                record.filling = false;
                                true
                            } else { false }
                        } else { false }
                    };
                    if expired { publish_presence(&state, &campaign_id, number).await; }
                }
            }
        }
    }

    if let Some(number) = ornament_number {
        let mut presence = state.presence.lock().await;
        let key = (campaign_id.clone(), number);
        if let Some(record) = presence.get_mut(&key) {
            record.connections = record.connections.saturating_sub(1);
            if record.connections == 0 {
                presence.remove(&key);
            }
        }
        drop(presence);
        publish_presence(&state, &campaign_id, number).await;
    }
}

async fn presence_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let session = authenticated(&state, &headers).await?;
    if session.role != "admin" && session.role != "participant" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Недостаточно прав"));
    }
    Ok(ws
        .on_upgrade(move |socket| presence_connection(socket, state, session))
        .into_response())
}

async fn sse_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let session = authenticated(&state, &headers).await?;
    let is_admin = session.role == "admin";
    let campaign_id = session
        .campaign_id
        .unwrap_or_else(|| christmas_greetings::store::LEGACY_CAMPAIGN_ID.to_owned());
    let receiver = state.events.subscribe();
    let stream = stream::unfold(
        (receiver, campaign_id, is_admin),
        |(mut receiver, campaign_id, is_admin)| async move {
            loop {
                let event = match receiver.recv().await {
                    Ok(greeting) if is_admin || greeting.campaign_id == campaign_id => {
                        Event::default()
                            .event("greeting")
                            .json_data(greeting)
                            .unwrap_or_else(|_| Event::default().event("resync").data("snapshot"))
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        Event::default().event("resync").data("snapshot")
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                };
                return Some((
                    Ok::<Event, Infallible>(event),
                    (receiver, campaign_id, is_admin),
                ));
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let store = Store::connect().await?;
    let (events, _) = broadcast::channel(256);
    let feed_store = store.clone();
    let feed_events = events.clone();
    tokio::spawn(async move {
        feed_store.supervise_changefeed(feed_events).await;
    });
    let (presence_events, _) = broadcast::channel(512);
    let state = AppState {
        store,
        events,
        presence_events,
        presence: Arc::new(Mutex::new(HashMap::new())),
        cookie_secure: env::var("COOKIE_SECURE")
            .map(|v| v != "false")
            .unwrap_or(true),
    };
    let avatar_dir = env::var("AVATAR_DIR").unwrap_or_else(|_| "/data/avatars".to_owned());
    tokio::fs::create_dir_all(&avatar_dir).await?;
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/participant/exchange", post(participant_exchange))
        .route("/api/auth/admin/login", post(admin_login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/events", get(sse_events))
        .route("/api/presence", get(presence_ws))
        .route("/api/greetings", get(list_greetings).post(submit))
        .route("/api/greetings/{number}/like", post(toggle_greeting_like))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/campaign", get(campaign_summary))
        .route("/api/admin/campaign", post(admin_create_campaign))
        .route(
            "/api/admin/campaigns",
            get(admin_list_campaigns).post(admin_create_named_campaign),
        )
        .route(
            "/api/admin/campaigns/{campaign_id}",
            delete(admin_delete_campaign),
        )
        .route(
            "/api/admin/campaigns/{campaign_id}/participants",
            get(admin_campaign_participants),
        )
        .route(
            "/api/admin/campaigns/{campaign_id}/invitations",
            post(admin_add_campaign_invitations),
        )
        .route(
            "/api/admin/campaigns/{campaign_id}/invitations/{number}/reissue",
            post(admin_reissue_campaign_invitation),
        )
        .route(
            "/api/admin/campaigns/{campaign_id}/invitations/{number}/revoke",
            post(admin_revoke_campaign_invitation),
        )
        .route("/api/admin/participants", get(admin_participants))
        .route(
            "/api/admin/invitations/{number}/reissue",
            post(admin_reissue_invitation),
        )
        .route(
            "/api/admin/invitations/{number}/revoke",
            post(admin_revoke_invitation),
        )
        .nest_service("/avatars", ServeDir::new(&avatar_dir))
        .fallback_service(ServeDir::new("frontend").append_index_html_on_directories(true))
        .with_state(state)
        .layer(DefaultBodyLimit::max(7 * 1024 * 1024))
        .layer(TraceLayer::new_for_http());
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env::var("PORT").unwrap_or_else(|_| "8080".into()).parse()?;
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Congratis backend listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod campaign_validation_tests {
    use super::campaign_numbers;

    #[test]
    fn campaign_size_accepts_only_one_through_thirty_five() {
        assert_eq!(campaign_numbers(1).unwrap(), vec![1]);
        assert_eq!(campaign_numbers(35).unwrap().len(), 35);
        assert!(campaign_numbers(0).is_err());
        assert!(campaign_numbers(36).is_err());
    }

    #[test]
    fn campaign_counts_include_all_invitations_and_track_submissions() {
        let mut counts = HashMap::new();
        add_campaign_count(&mut counts, Some("campaign-a"), false);
        add_campaign_count(&mut counts, Some("campaign-a"), true);
        add_campaign_count(&mut counts, Some("campaign-b"), true);
        add_campaign_count(&mut counts, None, false);

        assert_eq!(counts.get("campaign-a"), Some(&(2, 1)));
        assert_eq!(counts.get("campaign-b"), Some(&(1, 1)));
        assert_eq!(
            counts.get(christmas_greetings::store::LEGACY_CAMPAIGN_ID),
            Some(&(1, 0))
        );
    }
}
