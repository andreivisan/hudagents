use std::{
    env,
    io::{Error, ErrorKind, Result},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hudagents_proto::BridgeMessage;
use serde::Deserialize;
use tokio::sync::broadcast::{self, error::RecvError};

#[derive(Clone)]
struct AppState {
    messages: broadcast::Sender<String>,
    phone_token: Arc<str>,
    next_seq: Arc<AtomicU64>,
}

#[derive(Deserialize)]
struct PhoneQuery {
    token: Option<String>,
}

async fn say(State(state): State<AppState>, request_body: String) -> StatusCode {
    tracing::info!(body_bytes = request_body.len(), "Received say request");
    let seq = state.next_seq.fetch_add(1, Ordering::Relaxed);
    let bridge_message = BridgeMessage::text(seq, request_body);
    match serde_json::to_string(&bridge_message) {
        Ok(json_string) => match state.messages.send(json_string) {
            Ok(_) => StatusCode::NO_CONTENT,
            Err(_) => StatusCode::SERVICE_UNAVAILABLE,
        },
        Err(error) => {
            tracing::error!(%error, "Failed to serialize message");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn phone(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    phone_query: Query<PhoneQuery>,
) -> Response {
    if phone_query.token.as_deref() != Some(state.phone_token.as_ref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let receiver = state.messages.subscribe();
    ws.on_upgrade(move |socket| handle_phone_socket(socket, receiver))
}

async fn handle_phone_socket(mut socket: WebSocket, mut receiver: broadcast::Receiver<String>) {
    tracing::info!("Web socket connected");
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) => {
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(%error, "WebSocket receive failed");
                        break;
                    }
                    None => {
                        break;
                    }
                };
            }
            outgoing = receiver.recv() => {
                match outgoing {
                    Ok(text) => {
                        if let Err(error) = socket.send(Message::Text(text.into())).await {
                            tracing::warn!(%error, "WebSocket send failed");
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped=skipped, "Total skipped");
                    }
                    Err(RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }
    tracing::info!("Web socket disconnected");
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
        next_seq: Arc::new(AtomicU64::new(1)),
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
