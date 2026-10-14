use std::{io, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: hudagents-device ws://<mac-ip>:8080/device",
        )
    })?;
    let (mut ws, _) = connect_async(url.as_str()).await?;
    ws.send(Message::Text(r#"{"type":"hello"}"#.into())).await?;
    let reply = timeout(IO_TIMEOUT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => return Ok(text),
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
    println!("server replied: {reply}");
    timeout(IO_TIMEOUT, ws.close(None))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket close timed out"))??;
    Ok(())
}
