use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::{env, net::SocketAddr};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState { pool: PgPool }

#[derive(Serialize)]
struct Greeting {
    ornament_number: i16,
    display_name: String,
    message: String,
}

#[derive(Deserialize)]
struct SubmitGreeting {
    token: String,
    display_name: String,
    message: String,
}

#[derive(Serialize)]
struct ApiMessage { message: &'static str }

struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ApiMessage { message: self.1 })).into_response()
    }
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    sqlx::query("SELECT 1").execute(&state.pool).await
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "database unavailable"))?;
    Ok((StatusCode::OK, Json(serde_json::json!({"status":"ok"}))))
}

async fn list_greetings(State(state): State<AppState>) -> Result<Json<Vec<Greeting>>, ApiError> {
    let rows = sqlx::query_as::<_, (i16, String, String)>(
        "SELECT ornament_number, display_name, message FROM greetings ORDER BY ornament_number"
    )
    .fetch_all(&state.pool).await
    .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось загрузить поздравления"))?;

    Ok(Json(rows.into_iter().map(|(ornament_number, display_name, message)| Greeting {
        ornament_number, display_name, message
    }).collect()))
}

async fn submit(
    State(state): State<AppState>,
    Json(payload): Json<SubmitGreeting>,
) -> Result<impl IntoResponse, ApiError> {
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

    let mut tx = state.pool.begin().await
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Ошибка базы данных"))?;

    // Atomic claim. Concurrent submissions with the same token cannot both succeed.
    let participant = sqlx::query_as::<_, (uuid::Uuid, i16)>(
        "UPDATE participants SET used_at = NOW() WHERE token_hash = $1 AND used_at IS NULL RETURNING id, ornament_number"
    )
    .bind(hash_token(token))
    .fetch_optional(&mut *tx).await
    .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Ошибка базы данных"))?;

    let Some((participant_id, ornament_number)) = participant else {
        tx.rollback().await.ok();
        return Err(ApiError(StatusCode::CONFLICT, "Эта персональная ссылка уже использована или недействительна"));
    };

    if let Err(error) = sqlx::query(
        "INSERT INTO greetings (participant_id, ornament_number, display_name, message) VALUES ($1, $2, $3, $4)"
    )
    .bind(participant_id)
    .bind(ornament_number)
    .bind(name)
    .bind(message)
    .execute(&mut *tx).await {
        tracing::error!(?error, "failed to insert greeting");
        tx.rollback().await.ok();
        return Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось сохранить поздравление"));
    }

    tx.commit().await
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось завершить сохранение"))?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({
        "message": "Поздравление сохранено",
        "ornament_number": ornament_number
    }))))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL is required; configure Supabase PostgreSQL connection string");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = AppState { pool };
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/greetings", get(list_greetings).post(submit))
        .fallback_service(ServeDir::new("frontend").append_index_html_on_directories(true))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env::var("PORT").unwrap_or_else(|_| "8080".into()).parse()?;
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Christmas Greetings listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
