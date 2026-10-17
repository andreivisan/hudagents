use std::{io, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpStream, time::timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use hudagents_proto::{
    MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION,
    device::{DeviceId, DeviceMessage, ErrorCode, ServerMsg},
};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
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

    #[error("Expected a JSON text reply from the server")]
    UnexpectedReply,

    #[error("Expected protocol version {PROTOCOL_VERSION}, got {received}")]
    UnsupportedVersion { received: u16 },

    #[error("Device handshake rejected ({code:?}): {message}")]
    Rejected { code: ErrorCode, message: String },
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
    };
    Ok(ws)
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
    let mut ws = connect_and_handshake(&url, &hello).await?;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = &mut shutdown => {
                result?;
                timeout(IO_TIMEOUT, ws.close(None))
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket close timed out"))??;
                println!("Device stopped");
                break;
            }
            incoming = ws.next() =>  {
                match incoming {
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => {
                        println!("Server disconnected");
                        break;
                    }
                    Some(Err(error)) => {
                        return Err(error.into());
                    }
                    Some(Ok(Message::Text(text))) => {
                        match ServerMsg::from_json(text.as_str())? {
                            ServerMsg::Error { code, message } => {
                                return Err(io::Error::other(format!(
                                    "Server reported an error ({code:?}): {message}"
                                ))
                                .into());
                            }
                            ServerMsg::Welcome { .. } => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Received a second welcome",
                                )
                                .into());
                            }
                        }
                    }
                    Some(Ok(_)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Unexpected WebSocket message",
                        )
                        .into());
                    }
                }
            }
        }
    }
    Ok(())
}
