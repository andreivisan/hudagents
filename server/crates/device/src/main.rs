use std::{io, time::Duration};

use futures_util::{SinkExt, StreamExt, stream::FuturesUnordered};
use tokio::{
    net::TcpStream,
    time::{self, sleep, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{
        Error as WsError, Message, error::ProtocolError as WsProtocolError,
        protocol::WebSocketConfig,
    },
};

use hudagents_proto::{
    MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION,
    device::{DeviceId, DeviceMessage, ErrorCode, MediaErrorCode, ServerMsg},
    media::{MediaFrame, MediaKind},
};

mod capture;
mod temperature;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
type DeviceSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    #[error("WebSocket failed: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Invalid server reply: {0}")]
    Protocol(#[from] hudagents_proto::device::ProtocolError),

    #[error("Timed out while {0}")]
    Timeout(&'static str),

    #[error("Server closed the connection before sending welcome")]
    Closed,

    #[error("Unexpected message from the server")]
    UnexpectedReply,

    #[error("Expected protocol version {PROTOCOL_VERSION}, got {received}")]
    UnsupportedVersion { received: u16 },

    #[error("Server reported an error ({code:?}): {message}")]
    Rejected { code: ErrorCode, message: String },
}

impl ConnectionError {
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_)
                | Self::Closed
                | Self::WebSocket(WsError::Io(_))
                | Self::WebSocket(WsError::ConnectionClosed)
                | Self::WebSocket(WsError::Protocol(WsProtocolError::HandshakeIncomplete))
                | Self::WebSocket(WsError::Protocol(
                    WsProtocolError::ResetWithoutClosingHandshake
                ))
        )
    }
}

async fn connect_and_handshake(url: &str, hello: &str) -> Result<DeviceSocket, ConnectionError> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_CONTROL_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_CONTROL_MESSAGE_BYTES));
    let (mut ws, _) = timeout(
        IO_TIMEOUT,
        connect_async_with_config(url, Some(config), false),
    )
    .await
    .map_err(|_| ConnectionError::Timeout("connecting"))??;
    timeout(IO_TIMEOUT, ws.send(Message::Text(hello.to_owned().into())))
        .await
        .map_err(|_| ConnectionError::Timeout("sending hello"))??;
    let reply = timeout(IO_TIMEOUT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    return ServerMsg::from_json(text.as_str()).map_err(ConnectionError::Protocol);
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) | None => {
                    return Err(ConnectionError::Closed);
                }
                Some(Ok(_)) => {
                    return Err(ConnectionError::UnexpectedReply);
                }
                Some(Err(error)) => return Err(error.into()),
            }
        }
    })
    .await
    .map_err(|_| ConnectionError::Timeout("waiting for welcome"))??;
    match reply {
        ServerMsg::Welcome { protocol_version } => {
            if protocol_version != PROTOCOL_VERSION {
                return Err(ConnectionError::UnsupportedVersion {
                    received: protocol_version,
                });
            }
            println!("Device handshake accepted");
        }
        ServerMsg::Error { code, message } => {
            return Err(ConnectionError::Rejected { code, message });
        }
        ServerMsg::Capture { .. } | ServerMsg::Record { .. } => {
            return Err(ConnectionError::UnexpectedReply);
        }
    };
    Ok(ws)
}

async fn send_device_message(
    ws: &mut DeviceSocket,
    message: DeviceMessage,
) -> Result<(), ConnectionError> {
    let device_message = DeviceMessage::to_json(&message)?;
    timeout(IO_TIMEOUT, ws.send(Message::Text(device_message.into())))
        .await
        .map_err(|_| ConnectionError::Timeout("sending device control message"))??;
    Ok(())
}

async fn run_session(ws: &mut DeviceSocket) -> Result<(), ConnectionError> {
    let mut interval = time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut temperature_read_failed = false;
    let mut captures = FuturesUnordered::new();

    loop {
        tokio::select! {
            received = ws.next() => {
                match received {
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => {
                        return Ok(());
                    }
                    Some(Err(error)) => {
                        return Err(error.into());
                    }
                    Some(Ok(Message::Text(text))) => match ServerMsg::from_json(text.as_str())? {
                        ServerMsg::Error { code, message } => {
                            return Err(ConnectionError::Rejected { code, message });
                        }
                        ServerMsg::Welcome { .. } => {
                            return Err(ConnectionError::UnexpectedReply);
                        }
                        ServerMsg::Capture { request_id, width, height, quality } => {
                            match capture::validate_capture(width, height, quality) {
                                Ok(()) => {
                                    if !captures.is_empty() {
                                        send_device_message(
                                            ws,
                                            DeviceMessage::CaptureFailed {
                                                request_id,
                                                code: MediaErrorCode::Busy,
                                                message: "Camera is busy capturing another request".into() })
                                        .await?;
                                        continue;
                                    } else {
                                        println!("Capture started: request_id={}", request_id.get());
                                        captures.push(async move {
                                            let result = capture::capture_jpeg(width, height, quality).await;
                                            (request_id, result)
                                        });
                                    }
                                }
                                Err(error) => {
                                    let device_message = DeviceMessage::CaptureFailed {
                                        request_id,
                                        code: error.code(),
                                        message: error.to_string(),
                                    };
                                    send_device_message(ws, device_message).await?;
                                    continue;
                                }
                            }
                        }
                        ServerMsg::Record { request_id, .. } => {
                            send_device_message(ws, DeviceMessage::RecordFailed {
                                request_id,
                                code: MediaErrorCode::Unavailable,
                                message: "Audio recording is not implemented on this device yet".into(),
                            }).await?;
                            continue;
                        }
                    },
                    _ => return Err(ConnectionError::UnexpectedReply),
                }
            }
            _ = interval.tick() => {
                match temperature::read_temperature().await {
                    Ok(temperature) => {
                        if temperature_read_failed {
                            println!("Temperature reading resumed");
                            temperature_read_failed = false;
                        }
                        let json = DeviceMessage::Telemetry {
                            temp_millicelsius: temperature,
                        }
                        .to_json()?;

                        timeout(
                            IO_TIMEOUT,
                            ws.send(Message::Text(json.into())),
                        )
                        .await
                        .map_err(|_| ConnectionError::Timeout("sending temperature telemetry"))??;
                    }
                    Err(error) => {
                        if !temperature_read_failed {
                            println!("Temperature reading unavailable: {error}");
                            temperature_read_failed = true;
                        }
                    }
                }
            }
            Some((request_id, result)) = captures.next(), if !captures.is_empty() => {
                match result {
                    Ok(jpeg) => {
                        let media_frame = MediaFrame {
                            kind: MediaKind::Jpeg,
                            request_id,
                            payload: &jpeg
                        };
                        match media_frame.encode() {
                            Ok(bytes) => {
                                timeout(IO_TIMEOUT, ws.send(Message::Binary(bytes.into())))
                                    .await
                                    .map_err(|_| ConnectionError::Timeout("sending captured JPEG"))??;
                                println!(
                                    "Capture sent: request_id={}, jpeg_bytes={}",
                                    request_id.get(),
                                    jpeg.len()
                                );
                            }
                            Err(error) => {
                                send_device_message(ws, DeviceMessage::CaptureFailed {
                                    request_id,
                                    code: MediaErrorCode::Io,
                                    message: format!("Failed to encode captured JPEG: {error}"),
                                }).await?;
                                continue;
                            }
                        };
                    }
                    Err(error) => {
                        eprintln!(
                            "Capture failed: request_id={}, error={error}",
                            request_id.get()
                        );
                        send_device_message(ws, DeviceMessage::CaptureFailed {
                            request_id,
                            code: error.code(),
                            message: error.to_string()
                        }).await?;
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: hudagents-device ws://<mac-ip>:8080/device",
        )
    })?;
    let raw_device_id = std::env::var("HUD_DEVICE_ID").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "HUD_DEVICE_ID must be set and contain valid Unicode",
        )
    })?;
    let device_id = DeviceId::try_from(raw_device_id).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid HUD_DEVICE_ID: {error}"),
        )
    })?;
    let hello = DeviceMessage::hello(device_id).to_json()?;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut retry_delay = INITIAL_RETRY_DELAY;
    loop {
        let connection = tokio::select! {
            biased;
            result = &mut shutdown => {
                result?;
                println!("Device stopped");
                return Ok(());
            }
            result = connect_and_handshake(&url, &hello) => result,
        };
        let outcome: Result<(), ConnectionError> = match connection {
            Err(error) => Err(error),
            Ok(mut ws) => {
                retry_delay = INITIAL_RETRY_DELAY;
                tokio::select! {
                    biased;
                    result = &mut shutdown => {
                        result?;
                        match timeout(IO_TIMEOUT, ws.close(None)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => eprintln!("WebSocket close failed: {error}"),
                            Err(_) => eprintln!("WebSocket close timed out"),
                        }
                        println!("Device stopped");
                        return Ok(());
                    }
                    result = run_session(&mut ws) => result,
                }
            }
        };

        match outcome {
            Ok(()) => println!("Server disconnected"),
            Err(error) if error.is_retryable() => eprintln!("Connection failed: {error}"),
            Err(error) => return Err(error.into()),
        }

        println!("Reconnecting in {} seconds", retry_delay.as_secs());
        tokio::select! {
            biased;
            result = &mut shutdown => {
                result?;
                println!("Device stopped");
                return Ok(());
            }
            _ = sleep(retry_delay) => {}
        }
        retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
    }
}
