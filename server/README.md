# HudAgents server

The initial transport server connects a display client to text broadcasts and accepts perception-device WebSocket messages. HudBridge, a macOS desktop app, is the first display client; a phone app can use the same `/phone` endpoint later.

## Workspace

This directory is an independent Cargo workspace, excluded from the repository's root workspace.

| Crate | Responsibility |
| --- | --- |
| `hudagents-proto` | Shared message types and JSON serialization contract. |
| `hudagents-device` | Placeholder binary for the future perception-device client. |
| `hudagents-server` | Axum/Tokio HTTP and WebSocket server. |

## Run

Use a Rust toolchain supporting edition 2024. Run from the repository root:

```bash
HUDAGENTS_TOKEN=dev-secret cargo run --manifest-path server/Cargo.toml -p hudagents-server
```

Or run from this directory:

```bash
HUDAGENTS_TOKEN=dev-secret cargo run -p hudagents-server
```

`HUDAGENTS_TOKEN` must be nonempty and valid Unicode; startup fails otherwise. `dev-secret` is an example development token. The server binds to `0.0.0.0:8080`, making it reachable through the machine's network interfaces.

## Endpoints

| Endpoint | Behavior |
| --- | --- |
| WebSocket `/phone?token=<token>` | Validates the shared token, then forwards new broadcast messages as JSON text frames. A missing or incorrect token on a valid handshake returns HTTP 401. |
| WebSocket `/device` | Logs incoming text/binary message types and byte lengths, plus connection events. Payload contents are not logged or forwarded. |
| HTTP `POST /say` | Accepts a raw UTF-8 text body, wraps it in a protocol message, and publishes it to display subscribers. |

`/say` returns HTTP 204 when the broadcast channel accepts the message, 503 when there are no subscribers, or 500 if serialization fails. A 204 does not acknowledge delivery to the app or glasses. Connecting a device does not create a display subscriber.

`/say` is not a WebSocket endpoint: connecting to it with `websocat` uses GET and returns HTTP 405. Ordinary HTTP requests to `/phone` without WebSocket handshake headers can be rejected before token validation.

## Message format and delivery

Display clients receive JSON such as:

```json
{"type":"text","seq":1,"id":"550e8400-e29b-41d4-a716-446655440000","body":"hello"}
```

- `type`: the protocol supports `text` and `status`; `/say` currently produces only `text`.
- `seq`: a shared counter starting at 1 on each server startup. Failed requests can leave gaps. Concurrent requests receive distinct numbers but can publish out of numerical order.
- `id`: a new UUID for each message, suitable for client deduplication. All recipients of one broadcast receive the same ID and JSON.
- `body`: the submitted text.

Serialization happens once before broadcasting. The broadcast channel has capacity 64. Slow subscribers that fall behind log a skipped-message count and continue with retained messages. There is no persistence or replay for new or reconnecting clients. A WebSocket send that exceeds five seconds terminates that connection. Incoming pings are handled by the WebSocket implementation while the connection is processed.

## Manual checks

Install `websocat` and have `curl` available. Start the server using the command above, then open a display connection in another terminal:

```bash
websocat 'ws://127.0.0.1:8080/phone?token=dev-secret'
```

Publish from a third terminal:

```bash
curl -i -d 'hello' http://127.0.0.1:8080/say
```

Expect HTTP 204 and a JSON text message in the display terminal. Repeat the request to check successive sequence numbers and different UUIDs. Connect a second display client to verify that both receive identical messages for each request.

Disconnect all display clients and repeat the POST: expect HTTP 503. Try a display connection with `token=wrong`: expect HTTP 401.

To exercise the device endpoint:

```bash
websocat 'ws://127.0.0.1:8080/device'
```

Type `hello` and press Enter. Expect a device log recording a text message of five bytes. Disconnect and check the disconnection log. Abruptly terminating a client can produce a connection-reset warning before the disconnection log.

These checks validate transport through a command-line client; they do not verify HudBridge, Bluetooth, or physical glasses. Normal sends do not exercise the stalled-send timeout path.

## Development checks

Run from this directory:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The current automated test covers protocol serialization. Server endpoint behavior is covered by the manual checks above.

## Current scope

This is a trusted-network development server. `/say` and `/device` currently have no authentication, and the server does not configure TLS. Only `/phone` checks the shared token.

Device messages are logged as metadata only; image/audio processing, agent orchestration, model calls, delivery acknowledgements, and reconnect logic in the future device client are not implemented yet.
