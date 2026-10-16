use std::{io, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use hudagents_proto::{
    MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION,
    device::{DeviceId, DeviceMessage, ServerMsg},
};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

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
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_CONTROL_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_CONTROL_MESSAGE_BYTES));
    let (mut ws, _) = connect_async_with_config(url.as_str(), Some(config), false).await?;
    ws.send(Message::Text(hello.into())).await?;
    let reply = timeout(IO_TIMEOUT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    return ServerMsg::from_json(text.as_str())
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) | None => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "server closed before sending a reply",
                    ));
                }
                Some(Ok(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected a text reply from the server",
                    ));
                }
                Some(Err(error)) => return Err(io::Error::other(error)),
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "server reply timed out"))??;
    match reply {
        ServerMsg::Welcome { protocol_version } => {
            if protocol_version != PROTOCOL_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("expected protocol version {PROTOCOL_VERSION} got {protocol_version}"),
                )
                .into());
            }
            println!("Device handshake accepted");
        }
        ServerMsg::Error { code, message } => {
            return Err(io::Error::other(format!(
                "Device handshake rejected ({code:?}): {message}"
            ))
            .into());
        }
    };
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
