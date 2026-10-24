use super::*;

struct IdleDevice {
    state: AppState,
    connection_id: u64,
    commands: mpsc::Receiver<ServerMsg>,
    _stop: oneshot::Receiver<()>,
}

impl IdleDevice {
    async fn new() -> Self {
        let (state, messages) = state_with_subscriber();
        drop(messages);
        let (stop_tx, stop_rx) = oneshot::channel();
        let (connection_id, commands) = register_idle_device(&state, stop_tx).await;
        Self {
            state,
            connection_id,
            commands,
            _stop: stop_rx,
        }
    }
}

async fn pending_id(state: &AppState) -> Option<RequestId> {
    state
        .active_device
        .lock()
        .await
        .as_ref()
        .and_then(|device| device.pending_capture.as_ref())
        .map(|pending| pending.request_id)
}

async fn wait_result(state: &AppState, ticket: CaptureTicket) -> CaptureResult {
    timeout(TEST_TIMEOUT, state.wait_for_capture(ticket))
        .await
        .expect("capture result did not resolve")
}

// These bytes exercise the protocol's boundary-marker checks, not JPEG decoding.
fn jpeg_payload(len: usize) -> Vec<u8> {
    assert!(len >= 4);
    let mut jpeg = vec![0; len];
    jpeg[..2].copy_from_slice(&[0xFF, 0xD8]);
    jpeg[len - 2..].copy_from_slice(&[0xFF, 0xD9]);
    jpeg
}

async fn send_media(client: &mut Client, request_id: RequestId, kind: MediaKind, payload: &[u8]) {
    let frame = MediaFrame {
        kind,
        request_id,
        payload,
    };
    send(
        client,
        ClientMessage::Binary(frame.encode().unwrap().into()),
    )
    .await;
}

async fn send_failure(client: &mut Client, request_id: RequestId) {
    let failure = DeviceMessage::CaptureFailed {
        request_id,
        code: MediaErrorCode::Unavailable,
        message: "Camera unavailable".into(),
    };
    send(
        client,
        ClientMessage::Text(failure.to_json().unwrap().into()),
    )
    .await;
}

async fn receive_capture(client: &mut Client) -> RequestId {
    loop {
        match receive(client).await {
            ClientMessage::Ping(payload) => {
                send(client, ClientMessage::Pong(payload)).await;
            }
            ClientMessage::Text(text) => match ServerMsg::from_json(&text).unwrap() {
                ServerMsg::Capture {
                    request_id,
                    width: 1920,
                    height: 1080,
                    quality: 85,
                } => return request_id,
                other => panic!("expected default capture command, received {other:?}"),
            },
            other => panic!("expected capture command, received {other:?}"),
        }
    }
}

async fn ping_barrier(client: &mut Client) {
    send(client, ClientMessage::Ping(vec![23].into())).await;
    assert_eq!(receive(client).await, ClientMessage::Pong(vec![23].into()));
}

fn bridge_url(server: &TestServer) -> String {
    format!(
        "{}/phone?token=test-token",
        server.url.strip_suffix("/device").unwrap()
    )
}

async fn connect_bridge(server: &TestServer) -> Client {
    timeout(TEST_TIMEOUT, connect_async(bridge_url(server)))
        .await
        .expect("bridge connection timed out")
        .expect("bridge upgrade failed")
        .0
}

async fn tap(bridge: &mut Client, number: u64) {
    let json = format!(r#"{{"type":"tap","id":"00000000-0000-4000-8000-{number:012x}","taps":1}}"#);
    send(bridge, ClientMessage::Text(json.into())).await;
}

async fn bridge_status(bridge: &mut Client, expected: &str) {
    match receive(bridge).await {
        ClientMessage::Text(text) => {
            let message: BridgeMessage = serde_json::from_str(&text).unwrap();
            assert_eq!(message.msg_type, MessageType::Status);
            assert_eq!(message.body, expected);
            assert!(!message.id.is_nil());
        }
        other => panic!("expected bridge status, received {other:?}"),
    }
}

#[tokio::test]
async fn capture_without_device_is_unavailable_and_does_not_allocate_an_id() {
    let (state, _messages) = state_with_subscriber();
    assert!(matches!(
        state.begin_capture().await,
        Err(CaptureRequestError::Unavailable)
    ));
    assert_eq!(state.next_request_id.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn begin_capture_reserves_one_slot_and_queues_the_matching_command() {
    let mut device = IdleDevice::new().await;
    let before = Instant::now();
    let ticket = device.state.begin_capture().await.unwrap();
    assert_eq!(ticket.connection_id, device.connection_id);
    assert_eq!(ticket.request_id, RequestId::new(1));
    assert!(ticket.started_at >= before && ticket.started_at <= Instant::now());
    assert_eq!(pending_id(&device.state).await, Some(ticket.request_id));
    assert_eq!(device.commands.try_recv().unwrap(), capture_command(1));

    assert!(matches!(
        device.state.begin_capture().await,
        Err(CaptureRequestError::Busy)
    ));
    assert_eq!(device.state.next_request_id.load(Ordering::Relaxed), 2);
    assert!(matches!(
        device.commands.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn full_command_queue_rolls_back_capture_reservation_and_can_recover() {
    let mut device = IdleDevice::new().await;
    let sender = command_sender(&device.state).await;
    for id in 10..14 {
        sender.try_send(capture_command(id)).unwrap();
    }
    assert!(matches!(
        device.state.begin_capture().await,
        Err(CaptureRequestError::QueueFull)
    ));
    assert_eq!(pending_id(&device.state).await, None);
    for id in 10..14 {
        assert_eq!(device.commands.try_recv().unwrap(), capture_command(id));
    }
    let ticket = device.state.begin_capture().await.unwrap();
    assert_eq!(ticket.request_id, RequestId::new(2));
    assert_eq!(device.commands.try_recv().unwrap(), capture_command(2));
}

#[tokio::test]
async fn closed_command_queue_rolls_back_capture_reservation() {
    let mut device = IdleDevice::new().await;
    device.commands.close();
    assert!(matches!(
        device.state.begin_capture().await,
        Err(CaptureRequestError::Unavailable)
    ));
    assert_eq!(pending_id(&device.state).await, None);
}

#[tokio::test]
async fn completion_checks_both_ids_delivers_bytes_once_and_releases_the_slot() {
    let device = IdleDevice::new().await;
    let ticket = device.state.begin_capture().await.unwrap();
    let id = ticket.request_id;
    let jpeg = Bytes::from(jpeg_payload(16));
    let original_ptr = jpeg.as_ptr();
    assert!(
        !device
            .state
            .complete_capture(device.connection_id + 1, id, Ok(jpeg.clone()))
            .await
    );
    assert!(
        !device
            .state
            .complete_capture(device.connection_id, RequestId::new(999), Ok(jpeg.clone()))
            .await
    );
    assert_eq!(pending_id(&device.state).await, Some(id));
    assert!(
        device
            .state
            .complete_capture(device.connection_id, id, Ok(jpeg.clone()))
            .await
    );
    assert_eq!(pending_id(&device.state).await, None);
    let received = wait_result(&device.state, ticket).await.unwrap();
    assert_eq!(received, jpeg);
    assert_eq!(received.as_ptr(), original_ptr);
    assert!(
        !device
            .state
            .complete_capture(device.connection_id, id, Ok(jpeg))
            .await
    );
    assert_ne!(device.state.begin_capture().await.unwrap().request_id, id);
}

#[tokio::test]
async fn a_closed_result_receiver_still_releases_the_matching_capture() {
    let device = IdleDevice::new().await;
    let ticket = device.state.begin_capture().await.unwrap();
    let id = ticket.request_id;
    drop(ticket);
    assert!(
        device
            .state
            .complete_capture(
                device.connection_id,
                id,
                Err(CaptureRequestError::InvalidJpeg)
            )
            .await
    );
    assert_eq!(pending_id(&device.state).await, None);
    assert!(device.state.begin_capture().await.is_ok());
}

#[tokio::test]
async fn clear_capture_checks_both_ids_and_cannot_clear_a_later_request() {
    let device = IdleDevice::new().await;
    let first = device.state.begin_capture().await.unwrap();
    let first_id = first.request_id;
    assert!(
        !device
            .state
            .clear_pending_capture(device.connection_id + 1, first_id)
            .await
    );
    assert!(
        !device
            .state
            .clear_pending_capture(device.connection_id, RequestId::new(999))
            .await
    );
    assert_eq!(pending_id(&device.state).await, Some(first_id));
    assert!(
        device
            .state
            .clear_pending_capture(device.connection_id, first_id)
            .await
    );
    assert!(matches!(
        wait_result(&device.state, first).await,
        Err(CaptureRequestError::Disconnected)
    ));
    let next = device.state.begin_capture().await.unwrap();
    assert!(
        !device
            .state
            .clear_pending_capture(device.connection_id, first_id)
            .await
    );
    assert_eq!(pending_id(&device.state).await, Some(next.request_id));
}

#[tokio::test]
async fn unregister_and_replacement_resolve_waiters_without_affecting_new_requests() {
    let device = IdleDevice::new().await;
    let first = device.state.begin_capture().await.unwrap();
    device.state.unregister_device(device.connection_id).await;
    assert!(matches!(
        wait_result(&device.state, first).await,
        Err(CaptureRequestError::Disconnected)
    ));
    assert!(
        !device
            .state
            .clear_pending_capture(device.connection_id, RequestId::new(1))
            .await
    );

    let (second_stop_tx, _second_stop_rx) = oneshot::channel();
    let (second_connection, _second_commands) =
        register_idle_device(&device.state, second_stop_tx).await;
    let second = device.state.begin_capture().await.unwrap();
    let second_id = second.request_id;
    let (third_stop_tx, _third_stop_rx) = oneshot::channel();
    let (third_connection, _third_commands) =
        register_idle_device(&device.state, third_stop_tx).await;
    let third = device.state.begin_capture().await.unwrap();
    assert!(matches!(
        wait_result(&device.state, second).await,
        Err(CaptureRequestError::Disconnected)
    ));
    assert!(
        !device
            .state
            .clear_pending_capture(second_connection, second_id)
            .await
    );
    assert!(
        !device
            .state
            .complete_capture(second_connection, second_id, Ok(Bytes::new()))
            .await
    );
    device.state.unregister_device(second_connection).await;
    assert_eq!(
        active_connection_id(&device.state).await,
        Some(third_connection)
    );
    assert_eq!(pending_id(&device.state).await, Some(third.request_id));
}

#[tokio::test]
async fn wait_uses_the_original_deadline_and_timeout_releases_the_slot() {
    let mut device = IdleDevice::new().await;
    let mut ticket = device.state.begin_capture().await.unwrap();
    let old_id = ticket.request_id;
    device.commands.try_recv().unwrap();
    // An already-expired original deadline must not start another 15 s wait.
    ticket.started_at -= CAPTURE_REQUEST_TIMEOUT;
    assert!(matches!(
        wait_result(&device.state, ticket).await,
        Err(CaptureRequestError::Timeout)
    ));
    assert_eq!(pending_id(&device.state).await, None);
    let next = device.state.begin_capture().await.unwrap();
    assert_ne!(next.request_id, old_id);
    assert!(
        !device
            .state
            .complete_capture(device.connection_id, old_id, Ok(Bytes::new()))
            .await
    );
    assert_eq!(pending_id(&device.state).await, Some(next.request_id));
}

#[tokio::test]
async fn camera_errors_reach_the_waiter_unchanged() {
    let device = IdleDevice::new().await;
    let ticket = device.state.begin_capture().await.unwrap();
    assert!(
        device
            .state
            .complete_capture(
                device.connection_id,
                ticket.request_id,
                Err(CaptureRequestError::DeviceFailed {
                    code: MediaErrorCode::Unavailable,
                    message: "Camera unavailable".into(),
                }),
            )
            .await
    );
    assert!(matches!(
        wait_result(&device.state, ticket).await,
        Err(CaptureRequestError::DeviceFailed { code: MediaErrorCode::Unavailable, message })
            if message == "Camera unavailable"
    ));
}

#[tokio::test]
async fn jpeg_frames_above_control_limit_and_at_media_limit_complete_capture() {
    let (server, mut messages) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    for size in [
        MAX_CONTROL_MESSAGE_BYTES + 1,
        MAX_BINARY_MESSAGE_BYTES - MEDIA_HEADER_BYTES,
    ] {
        let ticket = server.state.begin_capture().await.unwrap();
        assert_eq!(receive_capture(&mut client).await, ticket.request_id);
        let jpeg = jpeg_payload(size);
        send_media(&mut client, ticket.request_id, MediaKind::Jpeg, &jpeg).await;
        let received = wait_result(&server.state, ticket).await.unwrap();
        assert_eq!(received.len(), size);
        assert_eq!(received.as_ref(), jpeg.as_slice());
        assert_eq!(pending_id(&server.state).await, None);
    }
    ping_barrier(&mut client).await;
    assert_no_statuses(&mut messages);
    send(&mut client, ClientMessage::Close(None)).await;
    expect_status(&mut messages, "Camera unit offline", 2).await;
}

#[tokio::test]
async fn bad_jpeg_payload_fails_the_request_and_the_same_connection_recovers() {
    let (server, mut messages) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    for payload in [
        &[0][..],
        &[0xFF, 0xD8, 0xD9],
        &[0, 0, 0xFF, 0xD9],
        &[0xFF, 0xD8, 0, 0],
    ] {
        let ticket = server.state.begin_capture().await.unwrap();
        assert_eq!(receive_capture(&mut client).await, ticket.request_id);
        send_media(&mut client, ticket.request_id, MediaKind::Jpeg, payload).await;
        assert!(matches!(
            wait_result(&server.state, ticket).await,
            Err(CaptureRequestError::InvalidJpeg)
        ));
        assert_eq!(pending_id(&server.state).await, None);
    }
    let ticket = server.state.begin_capture().await.unwrap();
    assert_eq!(receive_capture(&mut client).await, ticket.request_id);
    send_media(
        &mut client,
        ticket.request_id,
        MediaKind::Jpeg,
        &jpeg_payload(32),
    )
    .await;
    assert_eq!(wait_result(&server.state, ticket).await.unwrap().len(), 32);
    ping_barrier(&mut client).await;
    assert_no_statuses(&mut messages);
    send(&mut client, ClientMessage::Close(None)).await;
    expect_status(&mut messages, "Camera unit offline", 2).await;
}

#[tokio::test]
async fn late_jpeg_and_capture_failure_cannot_complete_a_newer_request() {
    let (server, mut messages) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let mut first = server.state.begin_capture().await.unwrap();
    let old_id = receive_capture(&mut client).await;
    assert_eq!(old_id, first.request_id);
    first.started_at -= CAPTURE_REQUEST_TIMEOUT;
    assert!(matches!(
        wait_result(&server.state, first).await,
        Err(CaptureRequestError::Timeout)
    ));
    let second = server.state.begin_capture().await.unwrap();
    assert_eq!(receive_capture(&mut client).await, second.request_id);
    send_media(&mut client, old_id, MediaKind::Jpeg, &jpeg_payload(8)).await;
    send_failure(&mut client, old_id).await;
    ping_barrier(&mut client).await;
    assert_eq!(pending_id(&server.state).await, Some(second.request_id));
    assert_no_statuses(&mut messages);

    send_media(
        &mut client,
        second.request_id,
        MediaKind::Jpeg,
        &jpeg_payload(16),
    )
    .await;
    assert_eq!(wait_result(&server.state, second).await.unwrap().len(), 16);
    send(&mut client, ClientMessage::Close(None)).await;
    expect_status(&mut messages, "Camera unit offline", 2).await;
}

#[tokio::test]
async fn malformed_media_and_unrequested_wav_close_and_release_the_pending_waiter() {
    let frame = MediaFrame {
        kind: MediaKind::Wav,
        request_id: RequestId::new(1),
        payload: &[0],
    }
    .encode()
    .unwrap();
    let mut bad_version = frame.clone();
    bad_version[0] = 2;
    let mut bad_kind = frame.clone();
    bad_kind[1] = 99;
    for (bytes, code) in [
        (bad_version, ErrorCode::InvalidMessage),
        (bad_kind, ErrorCode::InvalidMessage),
        (
            frame[..MEDIA_HEADER_BYTES].to_vec(),
            ErrorCode::InvalidMessage,
        ),
        (frame, ErrorCode::UnexpectedMessage),
    ] {
        let (server, mut messages) = TestServer::start().await;
        let mut client = server.connect().await;
        send_hello(&mut client).await;
        expect_status(&mut messages, "Camera unit online", 1).await;
        let ticket = server.state.begin_capture().await.unwrap();
        assert_eq!(receive_capture(&mut client).await, ticket.request_id);
        send(&mut client, ClientMessage::Binary(bytes.into())).await;
        assert_error_and_close(&mut client, code).await;
        expect_status(&mut messages, "Camera unit offline", 2).await;
        assert!(matches!(
            wait_result(&server.state, ticket).await,
            Err(CaptureRequestError::Disconnected)
        ));
        assert_eq!(active_connection_id(&server.state).await, None);
    }
}

#[tokio::test]
async fn media_message_one_byte_over_limit_disconnects_and_releases_waiter() {
    let (server, mut messages) = TestServer::start().await;
    let mut client = server.connect().await;
    send_hello(&mut client).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let ticket = server.state.begin_capture().await.unwrap();
    receive_capture(&mut client).await;
    // The peer may reject the frame header before the client finishes writing.
    let _ = timeout(
        TEST_TIMEOUT,
        client.send(ClientMessage::Binary(
            vec![0; MAX_BINARY_MESSAGE_BYTES + 1].into(),
        )),
    )
    .await
    .expect("oversized media send did not finish");
    expect_status(&mut messages, "Camera unit offline", 2).await;
    assert!(matches!(
        wait_result(&server.state, ticket).await,
        Err(CaptureRequestError::Disconnected)
    ));
    let reply = timeout(TEST_TIMEOUT, client.next()).await.unwrap();
    assert!(matches!(
        reply,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
    assert_eq!(active_connection_id(&server.state).await, None);
}

#[tokio::test]
async fn tap_capture_round_trip_preserves_bridge_pings_telemetry_and_busy_handling() {
    let (server, mut messages) = TestServer::start().await;
    let mut device = server.connect().await;
    send_hello(&mut device).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let mut bridge = connect_bridge(&server).await;

    tap(&mut bridge, 1).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let first = receive_capture(&mut device).await;
    // This would fail if the phone handler awaited the capture inline.
    ping_barrier(&mut bridge).await;
    tap(&mut bridge, 2).await;
    bridge_status(&mut bridge, "Camera is busy capturing another request").await;
    assert_eq!(pending_id(&server.state).await, Some(first));
    send_telemetry(&mut device, 53_250).await;
    let ClientMessage::Text(telemetry) = receive(&mut bridge).await else {
        panic!("expected forwarded telemetry");
    };
    let value: serde_json::Value = serde_json::from_str(&telemetry).unwrap();
    assert_eq!(value["type"], "telemetry");
    assert_eq!(value["temp_c"], 53.25);
    send_media(&mut device, first, MediaKind::Jpeg, &jpeg_payload(8192)).await;
    bridge_status(&mut bridge, "Photo received").await;
    assert_eq!(pending_id(&server.state).await, None);

    // A second completed request uses the same sockets and a fresh request ID.
    tap(&mut bridge, 3).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let second = receive_capture(&mut device).await;
    assert_ne!(second, first);
    send_media(&mut device, second, MediaKind::Jpeg, &jpeg_payload(32)).await;
    bridge_status(&mut bridge, "Photo received").await;
    ping_barrier(&mut bridge).await;
    ping_barrier(&mut device).await;
    send(&mut device, ClientMessage::Close(None)).await;
    bridge_status(&mut bridge, "Camera unit offline").await;
    send(&mut bridge, ClientMessage::Close(None)).await;
}

#[tokio::test]
async fn tap_camera_failure_is_reported_and_the_next_tap_can_succeed() {
    let (server, mut messages) = TestServer::start().await;
    let mut device = server.connect().await;
    send_hello(&mut device).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let mut bridge = connect_bridge(&server).await;
    tap(&mut bridge, 1).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let id = receive_capture(&mut device).await;
    send_failure(&mut device, id).await;
    bridge_status(
        &mut bridge,
        "Camera capture failed (Unavailable): Camera unavailable",
    )
    .await;
    assert_eq!(pending_id(&server.state).await, None);
    ping_barrier(&mut device).await;

    tap(&mut bridge, 2).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let id = receive_capture(&mut device).await;
    send_media(&mut device, id, MediaKind::Jpeg, &jpeg_payload(32)).await;
    bridge_status(&mut bridge, "Photo received").await;
    send(&mut device, ClientMessage::Close(None)).await;
    bridge_status(&mut bridge, "Camera unit offline").await;
    send(&mut bridge, ClientMessage::Close(None)).await;
}

#[tokio::test]
async fn offline_tap_is_reported_without_disconnecting_bridge() {
    let (server, mut messages) = TestServer::start().await;
    let mut bridge = connect_bridge(&server).await;
    tap(&mut bridge, 1).await;
    bridge_status(&mut bridge, "Perception device is offline").await;
    ping_barrier(&mut bridge).await;
    assert_eq!(server.state.next_request_id.load(Ordering::Relaxed), 1);

    let mut device = server.connect().await;
    send_hello(&mut device).await;
    // The first published event was the offline-tap error.
    expect_status(&mut messages, "Perception device is offline", 1).await;
    expect_status(&mut messages, "Camera unit online", 2).await;
    bridge_status(&mut bridge, "Camera unit online").await;
    tap(&mut bridge, 2).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let id = receive_capture(&mut device).await;
    send_media(&mut device, id, MediaKind::Jpeg, &jpeg_payload(32)).await;
    bridge_status(&mut bridge, "Photo received").await;
    send(&mut device, ClientMessage::Close(None)).await;
    bridge_status(&mut bridge, "Camera unit offline").await;
    send(&mut bridge, ClientMessage::Close(None)).await;
}

#[tokio::test]
async fn invalid_bridge_events_do_not_trigger_capture_or_close_the_connection() {
    let (server, mut messages) = TestServer::start().await;
    let mut device = server.connect().await;
    send_hello(&mut device).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let mut bridge = connect_bridge(&server).await;
    for input in [
        "garbage",
        r#"{"type":"tap","id":"bad-id","taps":1}"#,
        r#"{"type":"tap","taps":1}"#,
        r#"{"type":"tap","id":"00000000-0000-4000-8000-000000000001"}"#,
        r#"{"type":"tap","id":"00000000-0000-4000-8000-000000000001","taps":"1"}"#,
        r#"{"type":"tap","id":"00000000-0000-4000-8000-000000000001","taps":0}"#,
        r#"{"type":"tap","id":"00000000-0000-4000-8000-000000000001","taps":2}"#,
        r#"{"type":"tap","id":"00000000-0000-4000-8000-000000000001","taps":256}"#,
        r#"{"type":"capture","id":"00000000-0000-4000-8000-000000000001","taps":1}"#,
    ] {
        send(&mut bridge, ClientMessage::Text(input.into())).await;
        ping_barrier(&mut bridge).await;
        assert_eq!(pending_id(&server.state).await, None);
        assert_eq!(server.state.next_request_id.load(Ordering::Relaxed), 1);
        assert_no_statuses(&mut messages);
    }
    tap(&mut bridge, 1).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let id = receive_capture(&mut device).await;
    send_media(&mut device, id, MediaKind::Jpeg, &jpeg_payload(32)).await;
    bridge_status(&mut bridge, "Photo received").await;
    send(&mut device, ClientMessage::Close(None)).await;
    bridge_status(&mut bridge, "Camera unit offline").await;
    send(&mut bridge, ClientMessage::Close(None)).await;
}

#[tokio::test]
async fn bridge_upgrade_rejects_a_wrong_token() {
    let (server, _messages) = TestServer::start().await;
    let url = bridge_url(&server).replace("token=test-token", "token=wrong");
    let reply = timeout(TEST_TIMEOUT, connect_async(url)).await.unwrap();
    assert!(matches!(
        reply,
        Err(tokio_tungstenite::tungstenite::Error::Http(response))
            if response.status() == StatusCode::UNAUTHORIZED
    ));
    assert_eq!(server.state.next_request_id.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn bridge_disconnect_does_not_cancel_the_running_capture_task() {
    let (server, mut messages) = TestServer::start().await;
    let mut device = server.connect().await;
    send_hello(&mut device).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let mut bridge = connect_bridge(&server).await;
    tap(&mut bridge, 1).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    expect_status(&mut messages, "Taking picture...", 2).await;
    let id = receive_capture(&mut device).await;
    drop(bridge);
    send_media(&mut device, id, MediaKind::Jpeg, &jpeg_payload(32)).await;
    expect_status(&mut messages, "Photo received", 3).await;
    assert_eq!(pending_id(&server.state).await, None);
    assert!(server.state.begin_capture().await.is_ok());
    send(&mut device, ClientMessage::Close(None)).await;
    expect_status(&mut messages, "Camera unit offline", 4).await;
}

#[tokio::test]
#[ignore = "real-time tap/capture timeout recovery: about 15 seconds"]
async fn tap_timeout_reports_failure_and_the_next_tap_can_succeed() {
    let (server, mut messages) = TestServer::start().await;
    let mut device = server.connect().await;
    send_hello(&mut device).await;
    expect_status(&mut messages, "Camera unit online", 1).await;
    let mut bridge = connect_bridge(&server).await;
    tap(&mut bridge, 1).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let old_id = receive_capture(&mut device).await;
    ping_barrier(&mut bridge).await;
    let response = timeout(CAPTURE_REQUEST_TIMEOUT + TEST_TIMEOUT, bridge.next())
        .await
        .expect("capture timeout status did not arrive")
        .unwrap()
        .unwrap();
    let ClientMessage::Text(text) = response else {
        panic!("expected timeout status");
    };
    let message: BridgeMessage = serde_json::from_str(&text).unwrap();
    assert_eq!(message.msg_type, MessageType::Status);
    assert_eq!(message.body, "No photo received within 15 seconds");
    assert_eq!(pending_id(&server.state).await, None);
    tap(&mut bridge, 2).await;
    bridge_status(&mut bridge, "Taking picture...").await;
    let id = receive_capture(&mut device).await;
    assert_ne!(id, old_id);
    send_media(&mut device, id, MediaKind::Jpeg, &jpeg_payload(32)).await;
    bridge_status(&mut bridge, "Photo received").await;
    send(&mut device, ClientMessage::Close(None)).await;
    bridge_status(&mut bridge, "Camera unit offline").await;
    send(&mut bridge, ClientMessage::Close(None)).await;
}
