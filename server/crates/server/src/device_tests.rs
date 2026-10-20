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
    state: AppState,
}

impl TestServer {
    async fn start() -> (Self, broadcast::Receiver<String>) {
        let (state, receiver) = state_with_subscriber();
        let app = Router::new()
            .route("/device", get(device))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/device", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (Self { url, task, state }, receiver)
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

fn state_with_subscriber() -> (AppState, broadcast::Receiver<String>) {
    let (messages, receiver) = broadcast::channel(64);
    let state = AppState {
        active_device: Arc::new(Mutex::new(None)),
        next_connection_id: Arc::new(AtomicU64::new(1)),
        messages,
        phone_token: Arc::from("test-token"),
        next_seq: Arc::new(AtomicU64::new(1)),
    };
    (state, receiver)
}

fn camera_id() -> DeviceId {
    DeviceId::try_from("camera-01".to_owned()).unwrap()
}

async fn active_connection_id(state: &AppState) -> Option<u64> {
    state
        .active_device
        .lock()
        .await
        .as_ref()
        .map(|device| device.connection_id)
}

fn assert_no_statuses(receiver: &mut broadcast::Receiver<String>) {
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
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

async fn expect_status(
    receiver: &mut broadcast::Receiver<String>,
    body: &str,
    seq: u64,
) -> BridgeMessage {
    let status = receive_status(receiver).await;
    assert_eq!(status.msg_type, MessageType::Status);
    assert_eq!(status.body, body);
    assert_eq!(status.seq, seq);
    assert!(!status.id.is_nil());
    status
}

async fn send_hello(client: &mut Client) {
    send(client, ClientMessage::Text(HELLO.into())).await;
    assert_eq!(receive_reply(client).await, ServerMsg::welcome());
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

#[tokio::test]
async fn registration_stores_the_device_and_publishes_online() {
    let (state, mut statuses) = state_with_subscriber();
    let (stop_tx, mut stop_rx) = oneshot::channel();

    let connection_id = state.register_device(camera_id(), stop_tx).await;

    assert_eq!(connection_id, 1);
    {
        let active = state.active_device.lock().await;
        let device = active.as_ref().expect("device should be registered");
        assert_eq!(device.connection_id, connection_id);
        assert_eq!(device.device_id, camera_id());
    }
    expect_status(&mut statuses, "Camera unit online", 1).await;
    assert!(matches!(
        stop_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn replacement_signals_old_owner_and_ignores_its_late_cleanup() {
    let (state, mut statuses) = state_with_subscriber();
    let (first_tx, first_rx) = oneshot::channel();
    let first_id = state.register_device(camera_id(), first_tx).await;
    let first_online = expect_status(&mut statuses, "Camera unit online", 1).await;

    let (second_tx, mut second_rx) = oneshot::channel();
    let second_id = state.register_device(camera_id(), second_tx).await;
    let second_online = expect_status(&mut statuses, "Camera unit online", 2).await;
    assert!(second_id > first_id);
    assert_ne!(first_online.id, second_online.id);
    timeout(TEST_TIMEOUT, first_rx)
        .await
        .expect("previous owner was not signalled")
        .expect("stop sender was dropped without signalling");

    // Deliberately run the old cleanup after the replacement owns the slot.
    state.unregister_device(first_id).await;
    assert_eq!(active_connection_id(&state).await, Some(second_id));
    assert_no_statuses(&mut statuses);
    assert!(matches!(
        second_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    state.unregister_device(second_id).await;
    let offline = expect_status(&mut statuses, "Camera unit offline", 3).await;
    assert_ne!(offline.id, second_online.id);
    assert_eq!(active_connection_id(&state).await, None);

    // Repeated cleanup must not produce another offline notification.
    state.unregister_device(second_id).await;
    state.unregister_device(first_id).await;
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn unregistering_an_empty_slot_does_not_publish_offline() {
    let (state, mut statuses) = state_with_subscriber();
    state.unregister_device(42).await;
    assert_eq!(active_connection_id(&state).await, None);
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn replacement_succeeds_when_previous_stop_receiver_is_gone() {
    let (state, mut statuses) = state_with_subscriber();
    let (first_tx, first_rx) = oneshot::channel();
    state.register_device(camera_id(), first_tx).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;
    drop(first_rx);

    let (second_tx, _second_rx) = oneshot::channel();
    let second_id = state.register_device(camera_id(), second_tx).await;
    assert_eq!(active_connection_id(&state).await, Some(second_id));
    expect_status(&mut statuses, "Camera unit online", 2).await;
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn registration_and_cleanup_work_without_display_subscribers() {
    let (state, statuses) = state_with_subscriber();
    drop(statuses);
    let (stop_tx, _stop_rx) = oneshot::channel();

    let connection_id = state.register_device(camera_id(), stop_tx).await;
    assert_eq!(active_connection_id(&state).await, Some(connection_id));
    state.unregister_device(connection_id).await;
    assert_eq!(active_connection_id(&state).await, None);
}

#[tokio::test]
async fn connection_numbers_are_not_reused_after_disconnect() {
    let (state, mut statuses) = state_with_subscriber();
    let (first_tx, _first_rx) = oneshot::channel();
    let first_id = state.register_device(camera_id(), first_tx).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;
    state.unregister_device(first_id).await;
    expect_status(&mut statuses, "Camera unit offline", 2).await;

    let (second_tx, _second_rx) = oneshot::channel();
    let second_id = state.register_device(camera_id(), second_tx).await;
    assert!(second_id > first_id);
    expect_status(&mut statuses, "Camera unit online", 3).await;
    state.unregister_device(first_id).await;
    assert_eq!(active_connection_id(&state).await, Some(second_id));
    assert_no_statuses(&mut statuses);
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
    expect_status(&mut statuses, "Camera unit offline", 2).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
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
async fn second_hello_is_rejected_and_publishes_one_offline_status() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    send(&mut client, ClientMessage::Text(HELLO.into())).await;
    assert_eq!(receive_reply(&mut client).await, ServerMsg::welcome());
    receive_status(&mut statuses).await;

    send(&mut client, ClientMessage::Text(HELLO.into())).await;
    assert_error_and_close(&mut client, ErrorCode::UnexpectedMessage).await;
    expect_status(&mut statuses, "Camera unit offline", 2).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert!(matches!(
        statuses.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn replacement_closes_old_socket_without_publishing_false_offline() {
    let (server, mut statuses) = TestServer::start().await;
    let mut first = server.connect().await;
    send_hello(&mut first).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;
    let first_id = active_connection_id(&server.state).await.unwrap();

    let mut second = server.connect().await;
    send_hello(&mut second).await;
    expect_status(&mut statuses, "Camera unit online", 2).await;
    let second_id = active_connection_id(&server.state).await.unwrap();
    assert_ne!(first_id, second_id);
    assert!(matches!(receive(&mut first).await, ClientMessage::Close(_)));

    // The replacement must still respond after the previous socket is closed.
    let payload = vec![1, 2, 3];
    send(&mut second, ClientMessage::Ping(payload.clone().into())).await;
    assert_eq!(
        receive(&mut second).await,
        ClientMessage::Pong(payload.into())
    );
    assert_eq!(active_connection_id(&server.state).await, Some(second_id));
    assert_no_statuses(&mut statuses);

    send(&mut second, ClientMessage::Close(None)).await;
    expect_status(&mut statuses, "Camera unit offline", 3).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn invalid_hello_cannot_replace_an_existing_owner() {
    let (server, mut statuses) = TestServer::start().await;
    let mut owner = server.connect().await;
    send_hello(&mut owner).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;
    let owner_id = active_connection_id(&server.state).await;

    for (hello, code) in [
        ("garbage", ErrorCode::InvalidMessage),
        (
            r#"{"type":"hello","protocol_version":2,"device_id":"camera-01"}"#,
            ErrorCode::UnsupportedVersion,
        ),
    ] {
        let mut newcomer = server.connect().await;
        send(&mut newcomer, ClientMessage::Text(hello.into())).await;
        assert_error_and_close(&mut newcomer, code).await;
        assert_eq!(active_connection_id(&server.state).await, owner_id);
        assert_no_statuses(&mut statuses);
    }

    // Even a completed WebSocket upgrade does not acquire device ownership.
    let mut newcomer = server.connect().await;
    send(&mut newcomer, ClientMessage::Close(None)).await;
    send(&mut owner, ClientMessage::Ping(vec![7].into())).await;
    assert_eq!(
        receive(&mut owner).await,
        ClientMessage::Pong(vec![7].into())
    );
    assert_eq!(active_connection_id(&server.state).await, owner_id);
    assert_no_statuses(&mut statuses);

    send(&mut owner, ClientMessage::Close(None)).await;
    expect_status(&mut statuses, "Camera unit offline", 2).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn abrupt_socket_disconnect_cleans_up_the_owner() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;

    drop(client);

    expect_status(&mut statuses, "Camera unit offline", 2).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
}

#[tokio::test]
async fn binary_message_after_hello_closes_and_unregisters_the_owner() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;

    send(&mut client, ClientMessage::Binary(vec![1, 2, 3].into())).await;
    assert_error_and_close(&mut client, ErrorCode::InvalidMessage).await;

    expect_status(&mut statuses, "Camera unit offline", 2).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
}

// These exercise the actual 10 s / 30 s timers. Run explicitly with --ignored
// so routine protocol and ownership tests stay fast.
#[tokio::test]
#[ignore = "real-time heartbeat check: about 30 seconds"]
async fn silent_device_times_out_even_while_server_sends_pings() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    let started = Instant::now();
    send_hello(&mut client).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;

    // Keep the TCP socket open but never poll it again: tungstenite cannot
    // receive pings and send its automatic pongs.
    let json = timeout(SILENCE_TIMEOUT + TEST_TIMEOUT, statuses.recv())
        .await
        .expect("silent device did not time out")
        .unwrap();
    let offline: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(offline.msg_type, MessageType::Status);
    assert_eq!(offline.body, "Camera unit offline");
    assert_eq!(offline.seq, 2);
    assert!(started.elapsed() >= SILENCE_TIMEOUT);
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
    drop(client);
}

#[tokio::test]
#[ignore = "real-time heartbeat check: about 40 seconds"]
async fn pongs_keep_device_online_past_the_initial_silence_deadline() {
    let (server, mut statuses) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut statuses, "Camera unit online", 1).await;
    let connection_id = active_connection_id(&server.state).await;

    // Four server pings take us past the initial 30 s silence deadline.
    for _ in 0..4 {
        let message = timeout(PING_INTERVAL + TEST_TIMEOUT, client.next())
            .await
            .expect("server did not send its periodic ping")
            .expect("connection ended before ping")
            .expect("WebSocket receive failed");
        match message {
            ClientMessage::Ping(payload) => {
                assert!(payload.is_empty());
                send(&mut client, ClientMessage::Pong(payload)).await;
            }
            other => panic!("expected server ping, received {other:?}"),
        }
        assert_eq!(active_connection_id(&server.state).await, connection_id);
        assert_no_statuses(&mut statuses);
    }

    send(&mut client, ClientMessage::Close(None)).await;
    expect_status(&mut statuses, "Camera unit offline", 2).await;
    assert_eq!(active_connection_id(&server.state).await, None);
    assert_no_statuses(&mut statuses);
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
