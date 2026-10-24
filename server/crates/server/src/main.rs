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
    BridgeEvent, BridgeMessage, BridgeTelemetry, MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION,
    device::{DeviceId, DeviceMessage, ErrorCode, MediaErrorCode, RequestId, ServerMsg},
    media::{MAX_BINARY_MESSAGE_BYTES, MEDIA_HEADER_BYTES, MediaFrame, MediaKind},
};
use serde::Deserialize;
use tokio::{
    sync::{
        Mutex,
        broadcast::{self, error::RecvError},
        mpsc, oneshot,
    },
    time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout, timeout_at},
};

type CaptureResult = std::result::Result<Bytes, CaptureRequestError>;

const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_secs(10);
const SILENCE_TIMEOUT: Duration = Duration::from_secs(30);
const CAPTURE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
enum CaptureRequestError {
    #[error("Perception device is offline")]
    Unavailable,

    #[error("Camera is busy capturing another request")]
    Busy,

    #[error("Device command queue is full; try again")]
    QueueFull,

    #[error("Device connection ended before capture completed")]
    Disconnected,

    #[error("No photo received within 15 seconds")]
    Timeout,

    #[error("Camera capture failed ({code:?}): {message}")]
    DeviceFailed {
        code: MediaErrorCode,
        message: String,
    },

    #[error("Received empty or incomplete JPEG data")]
    InvalidJpeg,
}

#[derive(Debug)]
struct ActiveDevice {
    connection_id: u64,
    device_id: DeviceId,
    stop_tx: oneshot::Sender<()>,
    commands_tx: mpsc::Sender<ServerMsg>,
    pending_capture: Option<PendingCapture>,
}

#[derive(Clone)]
struct AppState {
    active_device: Arc<Mutex<Option<ActiveDevice>>>,
    next_connection_id: Arc<AtomicU64>,
    messages: broadcast::Sender<String>,
    phone_token: Arc<str>,
    next_seq: Arc<AtomicU64>,
    next_request_id: Arc<AtomicU64>,
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

    // Returns whether this reading belongs to the current connection, regardless of delivery.
    async fn publish_telemetry(&self, connection_id: u64, temp_millicelsius: i32) -> bool {
        let active_device = self.active_device.lock().await;
        match active_device.as_ref() {
            Some(ad) if ad.connection_id == connection_id => {}
            _ => return false,
        }

        let temp_c = f64::from(temp_millicelsius) / 1000.0;
        let next_seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let telemetry_message = BridgeTelemetry::new(next_seq, temp_c);
        match serde_json::to_string(&telemetry_message) {
            Ok(json_string) => match self.messages.send(json_string) {
                Ok(_) => {}
                Err(_) => {
                    tracing::debug!("Telemetry not broadcast: no display subscribers");
                }
            },
            Err(error) => {
                tracing::error!(connection_id, %error, "Failed to serialize telemetry message");
            }
        }

        // Keep ownership stable until this reading has been published.
        drop(active_device);
        true
    }

    async fn register_device(
        &self,
        device_id: DeviceId,
        stop_tx: oneshot::Sender<()>,
        commands_tx: mpsc::Sender<ServerMsg>,
    ) -> u64 {
        let mut active_device = self.active_device.lock().await;
        let next_connection_id = self.next_connection_id.fetch_add(1, Ordering::Relaxed);
        let new_active_device = ActiveDevice {
            connection_id: next_connection_id,
            device_id,
            stop_tx,
            commands_tx,
            pending_capture: None,
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

    async fn begin_capture(&self) -> std::result::Result<CaptureTicket, CaptureRequestError> {
        let mut active_device = self.active_device.lock().await;
        match active_device.as_mut() {
            Some(device) => {
                if device.pending_capture.is_some() {
                    return Err(CaptureRequestError::Busy);
                }
                let next_request_id =
                    RequestId::new(self.next_request_id.fetch_add(1, Ordering::Relaxed));
                let started_at = Instant::now();
                let (capture_tx, capture_rx) = oneshot::channel::<CaptureResult>();
                device.pending_capture = Some(PendingCapture {
                    request_id: next_request_id,
                    reply_tx: capture_tx,
                });
                let command = ServerMsg::Capture {
                    request_id: next_request_id,
                    width: 1920,
                    height: 1080,
                    quality: 85,
                };
                match device.commands_tx.try_send(command) {
                    Ok(()) => Ok(CaptureTicket {
                        connection_id: device.connection_id,
                        request_id: next_request_id,
                        started_at,
                        reply_rx: capture_rx,
                    }),
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        device.pending_capture.take();
                        Err(CaptureRequestError::QueueFull)
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        device.pending_capture.take();
                        Err(CaptureRequestError::Unavailable)
                    }
                }
            }
            None => Err(CaptureRequestError::Unavailable),
        }
    }

    async fn complete_capture(
        &self,
        connection_id: u64,
        request_id: RequestId,
        result: CaptureResult,
    ) -> bool {
        let mut active_device = self.active_device.lock().await;
        match active_device.as_mut() {
            Some(device) => {
                if device.connection_id != connection_id {
                    return false;
                }
                match device.pending_capture.as_ref() {
                    Some(pending) if pending.request_id != request_id => false,
                    Some(_) => match device.pending_capture.take() {
                        Some(pending) => match pending.reply_tx.send(result) {
                            Ok(()) => true,
                            Err(_) => {
                                tracing::debug!(
                                    connection_id,
                                    request_id = request_id.get(),
                                    "Capture result receiver already closed"
                                );
                                true
                            }
                        },
                        None => false,
                    },
                    None => false,
                }
            }
            None => false,
        }
    }

    async fn clear_pending_capture(&self, connection_id: u64, request_id: RequestId) -> bool {
        let mut active_device = self.active_device.lock().await;
        match active_device.as_mut() {
            Some(device) => {
                if device.connection_id != connection_id {
                    return false;
                }
                match device.pending_capture.as_ref() {
                    Some(pending) if pending.request_id != request_id => false,
                    Some(_) => {
                        device.pending_capture.take();
                        true
                    }
                    None => false,
                }
            }
            None => false,
        }
    }

    async fn wait_for_capture(&self, ticket: CaptureTicket) -> CaptureResult {
        let CaptureTicket {
            connection_id,
            request_id,
            started_at,
            reply_rx,
        } = ticket;
        match timeout_at(started_at + CAPTURE_REQUEST_TIMEOUT, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(CaptureRequestError::Disconnected),
            Err(_) => {
                self.clear_pending_capture(connection_id, request_id).await;
                Err(CaptureRequestError::Timeout)
            }
        }
    }
}

#[derive(Deserialize)]
struct PhoneQuery {
    token: Option<String>,
}

#[derive(Debug)]
struct PendingCapture {
    request_id: RequestId,
    reply_tx: oneshot::Sender<CaptureResult>,
}

#[derive(Debug)]
struct CaptureTicket {
    connection_id: u64,
    request_id: RequestId,
    started_at: Instant,
    reply_rx: oneshot::Receiver<CaptureResult>,
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
    ws.max_message_size(MAX_CONTROL_MESSAGE_BYTES)
        .max_frame_size(MAX_CONTROL_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_phone_socket(socket, receiver, state))
}

async fn handle_phone_socket(
    mut socket: WebSocket,
    mut receiver: broadcast::Receiver<String>,
    state: AppState,
) {
    tracing::info!("Phone connected");
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) => {
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<BridgeEvent>(text.as_str()) {
                            Ok(BridgeEvent::Tap { id, taps: 1 }) => {
                                tracing::info!(tap_id = %id, "Glasses tap received");
                                match state.begin_capture().await {
                                    Ok(ticket) => {
                                        let CaptureTicket {
                                            connection_id,
                                            request_id,
                                            started_at,
                                            ..
                                        } = ticket;
                                        tracing::info!(
                                            tap_id = %id,
                                            connection_id,
                                            request_id = request_id.get(),
                                            "Capture requested"
                                        );
                                        state.publish_status("Taking picture...");
                                        let capture_state = state.clone();
                                        let _capture_task = tokio::spawn(async move {
                                            match capture_state.wait_for_capture(ticket).await {
                                                Ok(jpeg) => {
                                                    tracing::info!(
                                                        tap_id = %id,
                                                        connection_id,
                                                        request_id = request_id.get(),
                                                        jpeg_bytes = jpeg.len(),
                                                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                                                        "Photo received from perception device"
                                                    );
                                                    capture_state.publish_status("Photo received");
                                                }
                                                Err(error) => {
                                                    tracing::warn!(
                                                        tap_id = %id,
                                                        connection_id,
                                                        request_id = request_id.get(),
                                                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                                                        %error,
                                                        "Capture request failed"
                                                    );
                                                    capture_state.publish_status(&error.to_string());
                                                }
                                            }
                                        });
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            tap_id = %id,
                                            %error,
                                            "Capture request rejected"
                                        );
                                        state.publish_status(&error.to_string());
                                    }
                                }
                            }
                            Ok(BridgeEvent::Tap { id, taps }) => {
                                tracing::warn!(
                                    tap_id = %id,
                                    taps,
                                    "Unsupported glasses tap count"
                                );
                            }
                            Err(error) => {
                                tracing::warn!(%error, "Invalid HudBridge event")
                            }
                        }
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
    ws.max_message_size(MAX_BINARY_MESSAGE_BYTES)
        .max_frame_size(MAX_BINARY_MESSAGE_BYTES)
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
        DeviceMessage::CaptureFailed { .. } | DeviceMessage::RecordFailed { .. } => {
            Err(ServerMsg::error(
                ErrorCode::UnexpectedMessage,
                "Expected hello before media message".to_string(),
            ))
        }
        DeviceMessage::Telemetry { .. } => Err(ServerMsg::error(
            ErrorCode::UnexpectedMessage,
            "Expected hello before telemetry".to_owned(),
        )),
    }
}

async fn handle_device_socket(mut socket: WebSocket, state: AppState) {
    tracing::info!("Device connected");
    let mut connection_id: Option<u64> = None;
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let mut stop_tx = Some(stop_tx);
    let (commands_tx, mut commands_rx) = mpsc::channel::<ServerMsg>(4);
    let mut commands_tx = Some(commands_tx);
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
            command = commands_rx.recv(), if connection_id.is_some() => {
                match command {
                    Some(cmd) => {
                        match cmd.to_json() {
                            Ok(json) => {
                                let deadline = silence.deadline().min(Instant::now() + SEND_TIMEOUT);
                                match timeout_at(deadline, socket.send(Message::Text(json.into()))).await {
                                    Ok(Ok(())) => continue,
                                    Ok(Err(error)) => {
                                        tracing::warn!(
                                            ?connection_id,
                                            %error,
                                            "Device command send failed"
                                        );
                                        break;
                                    }
                                    Err(_) => {
                                        tracing::warn!(
                                            ?connection_id,
                                            "Device command send timed out"
                                        );
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                tracing::error!(
                                    ?connection_id,
                                    %error,
                                    "Failed to serialize device command"
                                );
                                break;
                            }
                        }
                    }
                    None => {
                        tracing::debug!(?connection_id, "Device command channel closed");
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
                tracing::debug!(
                    message_type = "text",
                    body_bytes = text.len(),
                    "Device message received"
                );
                match (connection_id, DeviceMessage::from_json(text.as_str())) {
                    (_, Err(_)) => Err(ServerMsg::error(
                        ErrorCode::InvalidMessage,
                        "Invalid device control message".into(),
                    )),
                    (None, Ok(message)) => accept_hello(message),
                    (Some(current_id), Ok(DeviceMessage::Telemetry { temp_millicelsius })) => {
                        match temp_millicelsius {
                            -40_000..=150_000 => {
                                if state.publish_telemetry(current_id, temp_millicelsius).await {
                                    silence.as_mut().reset(Instant::now() + SILENCE_TIMEOUT);
                                }
                                continue;
                            }
                            _ => Err(ServerMsg::error(
                                ErrorCode::InvalidMessage,
                                format!(
                                    "Temperature must be between -40000 and 150000 millidegrees Celsius; received {temp_millicelsius}"
                                ),
                            )),
                        }
                    }
                    (Some(_), Ok(DeviceMessage::Hello { .. })) => Err(ServerMsg::error(
                        ErrorCode::UnexpectedMessage,
                        "Device has already completed its hello".to_string(),
                    )),
                    (
                        Some(current_id),
                        Ok(DeviceMessage::CaptureFailed {
                            request_id,
                            code,
                            message,
                        }),
                    ) => {
                        let result = Err(CaptureRequestError::DeviceFailed { code, message });
                        if state.complete_capture(current_id, request_id, result).await {
                            silence.as_mut().reset(Instant::now() + SILENCE_TIMEOUT);
                        } else {
                            tracing::debug!(
                                connection_id = current_id,
                                request_id = request_id.get(),
                                "Ignored unmatched capture failure"
                            );
                        }
                        continue;
                    }
                    (Some(_), Ok(DeviceMessage::RecordFailed { .. })) => Err(ServerMsg::error(
                        ErrorCode::UnexpectedMessage,
                        "Received a media failure without a pending request".into(),
                    )),
                }
            }
            Message::Binary(bytes) => {
                tracing::info!(
                    message_type = "bytes",
                    body_bytes = bytes.len(),
                    "Device message received"
                );
                match connection_id {
                    Some(current_id) => match MediaFrame::decode(&bytes) {
                        Ok(frame) => match frame.kind {
                            MediaKind::Wav => Err(ServerMsg::error(
                                ErrorCode::UnexpectedMessage,
                                "Received WAV without a pending recording request".into(),
                            )),
                            MediaKind::Jpeg => {
                                let request_id = frame.request_id;
                                let result: CaptureResult = if frame.payload.len() >= 4
                                    && frame.payload.starts_with(&[0xFF, 0xD8])
                                    && frame.payload.ends_with(&[0xFF, 0xD9])
                                {
                                    Ok(bytes.slice(MEDIA_HEADER_BYTES..))
                                } else {
                                    Err(CaptureRequestError::InvalidJpeg)
                                };

                                if state.complete_capture(current_id, request_id, result).await {
                                    silence.as_mut().reset(Instant::now() + SILENCE_TIMEOUT);
                                } else {
                                    tracing::debug!(
                                        connection_id = current_id,
                                        request_id = request_id.get(),
                                        "Ignored unmatched JPEG result"
                                    );
                                }
                                continue;
                            }
                        },
                        Err(error) => {
                            tracing::warn!(
                                connection_id = current_id,
                                %error,
                                "Invalid device media frame"
                            );

                            Err(ServerMsg::error(
                                ErrorCode::InvalidMessage,
                                "Invalid device media frame".into(),
                            ))
                        }
                    },
                    None => Err(ServerMsg::error(
                        ErrorCode::InvalidMessage,
                        "Expected hello before binary media".into(),
                    )),
                }
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
            connection_id = match (stop_tx.take(), commands_tx.take()) {
                (Some(stop_sender), Some(command_sender)) => {
                    Some(state.register_device(id, stop_sender, command_sender).await)
                }
                _ => {
                    tracing::error!(
                        ?connection_id,
                        "Missing channels during device registration"
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
        next_request_id: Arc::new(AtomicU64::new(1)),
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
            next_request_id: Arc::new(AtomicU64::new(1)),
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
