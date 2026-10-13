use std::{
    env,
    io::{Error, ErrorKind, Result},
    sync::Arc,
};

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use tokio::sync::broadcast;

#[derive(Clone)]
struct AppState {
    messages: broadcast::Sender<String>,
    phone_token: Arc<str>,
}

#[derive(Deserialize)]
struct PhoneQuery {
    token: Option<String>,
}

async fn say(State(state): State<AppState>, request_body: String) -> StatusCode {
    tracing::info!(body_bytes = request_body.len(), "Received say request");
    match state.messages.send(request_body) {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn phone(State(state): State<AppState>, phone_query: Query<PhoneQuery>) -> StatusCode {
    if phone_query.token.as_deref() != Some(state.phone_token.as_ref()) {
        return StatusCode::UNAUTHORIZED;
    }
    StatusCode::NO_CONTENT
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let (tx, rx) = broadcast::channel::<String>(64);
    drop(rx);
    let phone_token = env::var("HUDAGENTS_TOKEN").map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "HUDAGENTS_TOKEN must be set and contain valid Unicode",
        )
    })?;
    if phone_token.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "HUDAGENTS_TOKEN must not be empty",
        ));
    }
    let app_state = AppState {
        messages: tx,
        phone_token: phone_token.into(),
    };
    let app = Router::new()
        .route("/say", post(say))
        .route("/phone", get(phone))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "Server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
