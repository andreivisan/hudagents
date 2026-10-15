use super::*;
use futures_util::{SinkExt, StreamExt};
use hudagents_proto::MessageType;
use tokio::{net::TcpStream, task::JoinHandle};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message as ClientMessage,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);
const HELLO: &str = r#"{"type":"hello","protocol_version":1,"device_id":"camera-01"}"#;
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

// Each test gets its own loopback port and state, independent of a running dev server.
struct TestServer {
    url: String,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start() -> (Self, broadcast::Receiver<String>) {
        let (messages, receiver) = broadcast::channel(64);
        let state = AppState {
            messages,
            phone_token: Arc::from("test-token"),
            next_seq: Arc::new(AtomicU64::new(1)),
        };
        let app = Router::new()
            .route("/device", get(device))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/device", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (Self { url, task }, receiver)
    }

    async fn connect(&self) -> Client {
        timeout(TEST_TIMEOUT, connect_async(&self.url))
            .await
            .expect("connection timed out")
            .expect("WebSocket handshake failed")
            .0
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn send(client: &mut Client, message: ClientMessage) {
    timeout(TEST_TIMEOUT, client.send(message))
        .await
        .expect("send timed out")
        .expect("send failed");
}

async fn receive(client: &mut Client) -> ClientMessage {
    timeout(TEST_TIMEOUT, client.next())
        .await
        .expect("receive timed out")
        .expect("connection ended before expected message")
        .expect("WebSocket receive failed")
}

async fn receive_reply(client: &mut Client) -> ServerMsg {
    match receive(client).await {
        ClientMessage::Text(text) => ServerMsg::from_json(text.as_str()).unwrap(),
        other => panic!("expected JSON text reply, received {other:?}"),
    }
}

async fn receive_status(receiver: &mut broadcast::Receiver<String>) -> BridgeMessage {
    let json = timeout(TEST_TIMEOUT, receiver.recv())
        .await
        .expect("status broadcast timed out")
        .unwrap();
    serde_json::from_str(&json).unwrap()
}

async fn assert_error_and_close(client: &mut Client, expected_code: ErrorCode) {
    match receive_reply(client).await {
        ServerMsg::Error { code, message } => {
            assert_eq!(code, expected_code);
            assert!(!message.is_empty());
        }
        other => panic!("expected error reply, received {other:?}"),
    }
    assert!(matches!(receive(client).await, ClientMessage::Close(_)));
}

#[test]
fn accepts_current_protocol_version() {
    let id = DeviceId::try_from("camera-01".to_owned()).unwrap();
    assert_eq!(accept_hello(DeviceMessage::hello(id.clone())), Ok(id));
}

#[test]
fn rejects_unsupported_versions_with_the_expected_version() {
    for version in [0, 2, u16::MAX] {
        let message = DeviceMessage::Hello {
            protocol_version: version,
            device_id: DeviceId::try_from("camera-01".to_owned()).unwrap(),
        };
        assert_eq!(
            accept_hello(message),
            Err(ServerMsg::error(
                ErrorCode::UnsupportedVersion,
                format!("Expected protocol version 1, got {version}"),
            ))
        );
    }
}

#[tokio::test]
async fn valid_hello_at_size_limit_gets_welcome_and_one_online_status() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;

    // Control frames must not count as the first application message.
    let ping = vec![1, 2, 3];
    send(&mut client, ClientMessage::Ping(ping.clone().into())).await;
    assert_eq!(receive(&mut client).await, ClientMessage::Pong(ping.into()));
    assert!(matches!(
        statuses.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));

    let padded = format!(
        "{HELLO}{}",
        " ".repeat(MAX_CONTROL_MESSAGE_BYTES - HELLO.len())
    );
    send(&mut client, ClientMessage::Text(padded.into())).await;
    assert_eq!(receive_reply(&mut client).await, ServerMsg::welcome());
    let status = receive_status(&mut statuses).await;
    assert_eq!(status.msg_type, MessageType::Status);
    assert_eq!(status.body, "Camera unit online");
    assert_eq!(status.seq, 1);
    assert!(!status.id.is_nil());
    assert!(matches!(
        statuses.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    send(&mut client, ClientMessage::Close(None)).await;
}

#[tokio::test]
async fn invalid_handshakes_send_errors_then_close_without_online_status() {
    let (server, mut statuses) = TestServer::start().await;
    for (input, code) in [
        (
            r#"{"type":"hello","protocol_version":1,"device_id":"Camera-01"}"#,
            ErrorCode::InvalidMessage,
        ),
        (
            r#"{"type":"hello","protocol_version":2,"device_id":"camera-01"}"#,
            ErrorCode::UnsupportedVersion,
        ),
        (r#"{"type":"hello"}"#, ErrorCode::InvalidMessage),
        (
            r#"{"type":"welcome","protocol_version":1}"#,
            ErrorCode::InvalidMessage,
        ),
        ("garbage", ErrorCode::InvalidMessage),
    ] {
        let mut client = server.connect().await;
        send(&mut client, ClientMessage::Text(input.into())).await;
        assert_error_and_close(&mut client, code).await;
        assert!(matches!(
            statuses.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn second_hello_is_rejected_without_another_online_status() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    send(&mut client, ClientMessage::Text(HELLO.into())).await;
    assert_eq!(receive_reply(&mut client).await, ServerMsg::welcome());
    receive_status(&mut statuses).await;

    send(&mut client, ClientMessage::Text(HELLO.into())).await;
    assert_error_and_close(&mut client, ErrorCode::UnexpectedMessage).await;
    assert!(matches!(
        statuses.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn binary_message_is_rejected_without_online_status() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    send(
        &mut client,
        ClientMessage::Binary(HELLO.as_bytes().to_vec().into()),
    )
    .await;
    assert_error_and_close(&mut client, ErrorCode::InvalidMessage).await;
    assert!(matches!(
        statuses.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn welcome_does_not_require_a_display_subscriber() {
    let (server, statuses) = TestServer::start().await;
    drop(statuses);
    let mut client = server.connect().await;
    send(&mut client, ClientMessage::Text(HELLO.into())).await;
    assert_eq!(receive_reply(&mut client).await, ServerMsg::welcome());
    send(&mut client, ClientMessage::Close(None)).await;
}

#[tokio::test]
async fn oversized_hello_is_rejected_at_websocket_layer() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    let oversized = format!(
        "{HELLO}{}",
        " ".repeat(MAX_CONTROL_MESSAGE_BYTES + 1 - HELLO.len())
    );
    send(&mut client, ClientMessage::Text(oversized.into())).await;
    let response = timeout(TEST_TIMEOUT, client.next())
        .await
        .expect("oversized connection did not end");
    assert!(matches!(
        response,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
    assert!(matches!(
        statuses.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}
