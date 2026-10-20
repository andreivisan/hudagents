use std::{
    env,
    io::{Error, ErrorKind, Result},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hudagents_proto::{
    BridgeMessage, MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION,
    device::{DeviceId, DeviceMessage, ErrorCode, ServerMsg},
};
use serde::Deserialize;
use tokio::{
    sync::{
        Mutex,
        broadcast::{self, error::RecvError},
        oneshot,
    },
    time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout, timeout_at},
};

const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(10);
const SILENCE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct ActiveDevice {
    connection_id: u64,
    device_id: DeviceId,
    stop_tx: oneshot::Sender<()>,
}

#[derive(Clone)]
struct AppState {
    active_device: Arc<Mutex<Option<ActiveDevice>>>,
    next_connection_id: Arc<AtomicU64>,
    messages: broadcast::Sender<String>,
    phone_token: Arc<str>,
    next_seq: Arc<AtomicU64>,
}

impl AppState {
    fn publish_status(&self, body: &str) {
        let next_seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let bridge_status_message = BridgeMessage::status(next_seq, body.to_owned());
        match serde_json::to_string(&bridge_status_message) {
            Ok(json_string) => match self.messages.send(json_string) {
                Ok(_) => {}
                Err(_) => {
                    tracing::debug!("Status not broadcast: no display subscribers");
                }
            },
            Err(error) => {
                tracing::error!(%error, "Failed to serialize status message");
            }
        }
    }

    async fn register_device(&self, device_id: DeviceId, stop_tx: oneshot::Sender<()>) -> u64 {
        let mut active_device = self.active_device.lock().await;
        let next_connection_id = self.next_connection_id.fetch_add(1, Ordering::Relaxed);
        let new_active_device = ActiveDevice {
            connection_id: next_connection_id,
            device_id,
            stop_tx,
        };
        tracing::info!(
            connection_id = new_active_device.connection_id,
            device_id = new_active_device.device_id.as_str(),
            "Registering device connection"
        );
        if let Some(previous) = active_device.replace(new_active_device)
            && previous.stop_tx.send(()).is_err()
        {
            tracing::debug!(
                connection_id = previous.connection_id,
                "Previous device handler already stopped"
            );
        }
        // Publish before releasing the lock so ownership and status changes stay ordered.
        self.publish_status("Camera unit online");
        drop(active_device);
        next_connection_id
    }

    async fn unregister_device(&self, connection_id: u64) {
        let mut active_device = self.active_device.lock().await;
        match active_device.as_ref() {
            Some(present) => {
                if present.connection_id != connection_id {
                    return;
                }
            }
            None => {
                return;
            }
        }
        if let Some(ad) = active_device.take() {
            tracing::info!(
                connection_id = ad.connection_id,
                device_id = ad.device_id.as_str(),
                "Device connection unregistered"
            );
        }
        self.publish_status("Camera unit offline");
    }
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

fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    provided == Some(expected)
}

async fn phone(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    phone_query: Query<PhoneQuery>,
) -> Response {
    if !token_matches(phone_query.token.as_deref(), state.phone_token.as_ref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let receiver = state.messages.subscribe();
    ws.on_upgrade(move |socket| handle_phone_socket(socket, receiver))
}

async fn handle_phone_socket(mut socket: WebSocket, mut receiver: broadcast::Receiver<String>) {
    tracing::info!("Phone connected");
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
                        match timeout(
                            SEND_TIMEOUT,
                            socket.send(Message::Text(text.into())),
                         ).await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                tracing::warn!(%error, "WebSocket send failed");
                                break;
                            }
                            Err(_) => {
                                tracing::warn!("WebSocket send timeout");
                                break;
                            }
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
    tracing::info!("Phone disconnected");
}

async fn device(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.max_message_size(MAX_CONTROL_MESSAGE_BYTES)
        .max_frame_size(MAX_CONTROL_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_device_socket(socket, state))
}

fn accept_hello(message: DeviceMessage) -> std::result::Result<DeviceId, ServerMsg> {
    match message {
        DeviceMessage::Hello {
            protocol_version,
            device_id,
        } => {
            if protocol_version == PROTOCOL_VERSION {
                Ok(device_id)
            } else {
                Err(ServerMsg::error(
                    ErrorCode::UnsupportedVersion,
                    format!("Expected protocol version {PROTOCOL_VERSION}, got {protocol_version}"),
                ))
            }
        }
    }
}

async fn handle_device_socket(mut socket: WebSocket, state: AppState) {
    tracing::info!("Device connected");
    let mut connection_id: Option<u64> = None;
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let mut stop_tx = Some(stop_tx);
    let now = Instant::now();
    let mut ping_interval = interval_at(now + PING_INTERVAL, PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let silence = sleep_until(now + SILENCE_TIMEOUT);
    tokio::pin!(silence);
    loop {
        let message = tokio::select! {
            biased;
            _ = &mut stop_rx => {
                tracing::info!(?connection_id, "Device connection replaced");
                match timeout(SEND_TIMEOUT, socket.send(Message::Close(None))).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(?connection_id, %error, "Replaced device connection close failed");
                    }
                    Err(_) => {
                        tracing::warn!(?connection_id, "Replaced device connection close timed out");
                    }
                }
                break;
            }
            _ = &mut silence => {
                tracing::warn!("Device silence timeout");
                break;
            }
            _ = ping_interval.tick(), if connection_id.is_some() => {
                let deadline =
                        silence.deadline().min(Instant::now() + SEND_TIMEOUT);

                    match timeout_at(
                        deadline,
                        socket.send(Message::Ping(Bytes::new())),
                    )
                    .await
                    {
                        Ok(Ok(())) => continue,
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "Device ping send failed");
                            break;
                        }
                        Err(_) => {
                            tracing::warn!("Device ping send timed out");
                            break;
                        }
                    }
            }
            received = socket.recv() => {
                match received {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
                        tracing::warn!(%error, "Device receive failed");
                        break;
                    }
                    None =>  break,
                }
            }
        };
        // First decide the protocol outcome; sending and state changes happen below.
        let outcome = match message {
            Message::Text(text) => {
                tracing::info!(
                    message_type = "text",
                    body_bytes = text.len(),
                    "Device message received"
                );
                match DeviceMessage::from_json(text.as_str()) {
                    Err(_) => Err(ServerMsg::error(
                        ErrorCode::InvalidMessage,
                        "Expected a valid device hello".into(),
                    )),
                    Ok(_) if connection_id.is_some() => Err(ServerMsg::error(
                        ErrorCode::UnexpectedMessage,
                        "Device has already completed its hello".into(),
                    )),
                    Ok(message) => accept_hello(message),
                }
            }
            Message::Binary(bytes) => {
                tracing::info!(
                    message_type = "bytes",
                    body_bytes = bytes.len(),
                    "Device message received"
                );
                Err(ServerMsg::error(
                    ErrorCode::InvalidMessage,
                    "Expected a JSON text message; binary messages are not supported".into(),
                ))
            }
            Message::Ping(_) | Message::Pong(_) => {
                if connection_id.is_some() {
                    silence.as_mut().reset(Instant::now() + SILENCE_TIMEOUT);
                }
                continue;
            }
            Message::Close(_) => {
                break;
            }
        };

        let (reply, device_id) = match outcome {
            Ok(id) => (ServerMsg::welcome(), Some(id)),
            Err(error_reply) => (error_reply, None),
        };
        let json = match reply.to_json() {
            Ok(json) => json,
            Err(error) => {
                tracing::error!(%error, "Failed to serialize device reply");
                break;
            }
        };
        match timeout(SEND_TIMEOUT, socket.send(Message::Text(json.into()))).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(%error, "Device reply send failed");
                break;
            }
            Err(_) => {
                tracing::warn!("Device reply send timed out");
                break;
            }
        }

        // Only a successfully sent welcome commits the handshake and announces the device.
        if let Some(id) = device_id {
            tracing::info!(device_id = id.as_str(), "Device hello accepted");
            connection_id = match stop_tx.take() {
                Some(sender) => Some(state.register_device(id, sender).await),
                None => {
                    tracing::error!(
                        ?connection_id,
                        "Missing stop sender during device registration"
                    );
                    break;
                }
            };
            ping_interval.reset_at(Instant::now() + PING_INTERVAL);
            silence.as_mut().reset(Instant::now() + SILENCE_TIMEOUT);
        } else {
            match timeout(SEND_TIMEOUT, socket.send(Message::Close(None))).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Device close failed");
                }
                Err(_) => {
                    tracing::warn!("Device close timed out");
                }
            }
            break;
        }
    }
    if let Some(id) = connection_id {
        state.unregister_device(id).await;
    }
    tracing::info!(?connection_id, "Device disconnected");
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
        active_device: Arc::new(Mutex::new(None)),
        next_connection_id: Arc::new(AtomicU64::new(1)),
        messages: tx,
        phone_token: phone_token.into(),
        next_seq: Arc::new(AtomicU64::new(1)),
    };
    let app = Router::new()
        .route("/say", post(say))
        .route("/phone", get(phone))
        .route("/device", get(device))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "Server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod device_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_validation_rejects_missing_token() {
        assert!(!token_matches(None, "test-token"));
    }

    #[test]
    fn token_validation_rejects_incorrect_token() {
        for provided in [
            "wrong",
            "TEST-TOKEN",
            "test-token ",
            "test",
            "test-token-extra",
        ] {
            assert!(!token_matches(Some(provided), "test-token"));
        }
    }

    #[test]
    fn token_validation_rejects_empty_token() {
        assert!(!token_matches(Some(""), "test-token"));
    }

    #[test]
    fn token_validation_accepts_correct_token() {
        assert!(token_matches(Some("test-token"), "test-token"));
    }

    fn test_state() -> AppState {
        let (messages, receiver) = broadcast::channel(64);
        drop(receiver);
        AppState {
            active_device: Arc::new(Mutex::new(None)),
            next_connection_id: Arc::new(AtomicU64::new(1)),
            messages,
            phone_token: Arc::from("test-token"),
            next_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    #[tokio::test]
    async fn say_returns_unavailable_without_subscribers() {
        assert_eq!(
            say(State(test_state()), "hello".into()).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn say_publishes_json_with_original_body_and_unique_ids() {
        let state = test_state();
        let mut receiver = state.messages.subscribe();
        let body = "Hello \"glasses\"!\nSalut 👓";
        let mut previous_id = None;

        for expected_seq in 1..=2 {
            assert_eq!(
                say(State(state.clone()), body.into()).await,
                StatusCode::NO_CONTENT
            );
            let json = receiver.try_recv().expect("message should be published");
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["type"], "text");
            assert_eq!(value["seq"], expected_seq);
            assert_eq!(value["body"], body);
            assert!(value.get("msg_type").is_none());

            let message: BridgeMessage = serde_json::from_str(&json).unwrap();
            assert!(!message.id.is_nil());
            assert_ne!(previous_id, Some(message.id));
            previous_id = Some(message.id);
            assert!(matches!(
                receiver.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ));
        }
    }

    #[tokio::test]
    async fn say_delivers_identical_json_to_each_subscriber() {
        let state = test_state();
        let mut first = state.messages.subscribe();
        let mut second = state.messages.subscribe();
        assert_eq!(
            say(State(state), "hello".into()).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(first.try_recv().unwrap(), second.try_recv().unwrap());
    }

    #[tokio::test]
    async fn say_returns_unavailable_after_last_subscriber_disconnects() {
        let state = test_state();
        let receiver = state.messages.subscribe();
        assert_eq!(
            say(State(state.clone()), "connected".into()).await,
            StatusCode::NO_CONTENT
        );
        drop(receiver);
        assert_eq!(
            say(State(state), "disconnected".into()).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
