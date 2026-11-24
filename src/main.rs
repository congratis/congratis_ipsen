use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{convert::Infallible, env, net::SocketAddr, time::Duration};
use futures::stream;
use tokio::sync::broadcast;
use christmas_greetings::store::{GreetingEvent, Store};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;


#[derive(Clone)]
struct AppState {
    store: Store,
    events: broadcast::Sender<GreetingEvent>,
}

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

async fn events(
    State(state): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.events.subscribe();
    let stream = stream::unfold(receiver, |mut receiver| async move {
        let event = match receiver.recv().await {
            Ok(greeting) => Event::default()
                .event("greeting")
                .json_data(greeting)
                .unwrap_or_else(|_| Event::default().event("resync").data("snapshot")),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                Event::default().event("resync").data("snapshot")
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        };
        Some((Ok::<Event, Infallible>(event), receiver))
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    state
        .store
        .ping()
        .await
        .map_err(|error| {
            tracing::error!(?error, "RethinkDB health check failed");
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
        })?;

    Ok((StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))))
}

async fn list_greetings(
    State(state): State<AppState>,
) -> Result<Json<Vec<Greeting>>, ApiError> {
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
    Json(payload): Json<SubmitGreeting>,
) -> Result<impl IntoResponse, ApiError> {
    let token = payload.token.trim();
    let name = payload.display_name.trim();
    let message = payload.message.trim();

    if !(64..=128).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Некорректная персональная ссылка",
        ));
    }
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Имя должно содержать от 1 до 80 символов",
        ));
    }
    if message.is_empty() || message.chars().count() > 500 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "Поздравление должно содержать от 1 до 500 символов",
        ));
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

    let state = AppState { store, events };
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/events", get(events))
        .route("/api/greetings", get(list_greetings).post(submit))
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
    use super::hash_token;

    #[test]
    fn token_hash_is_stable_and_hex_encoded() {
        let token = "a".repeat(64);
        let first = hash_token(&token);
        let second = hash_token(&token);

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
