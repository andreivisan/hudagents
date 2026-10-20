# HudAgents server

The initial transport server connects a display client to text broadcasts and accepts perception-device WebSocket messages. HudBridge, a macOS desktop app, is the first display client; a phone app can use the same `/phone` endpoint later.

## Workspace

This directory is an independent Cargo workspace, excluded from the repository's root workspace.

| Crate | Responsibility |
| --- | --- |
| `hudagents-proto` | Shared control messages, display messages, and the media format described below. |
| `hudagents-device` | Raspberry Pi WebSocket client with a typed handshake and reconnect loop. |
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
| WebSocket `/device` | Accepts a typed device handshake, replies with `welcome`, and publishes online/offline display statuses. The server pings every 10 seconds and disconnects devices after 30 seconds of silence. Media handling is the next implementation step. |
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

## Device control and media protocol

Shared types live in `crates/proto/src/device.rs`. The binary codec belongs in
`crates/proto/src/media.rs`, exported by `pub mod media;` in the crate's `lib.rs`.

**Implementation status:** the handshake, connection lifecycle, capture/record
control types, and binary codec are implemented. The socket handlers do not yet
support capture, recording, or binary media. During this protocol checkpoint, their
matches also need updating for the new enum variants before the whole workspace
can compile.

### JSON control messages

Control messages use WebSocket text messages, capped at 4 KiB of UTF-8 JSON.
The first application message from the Pi must be `hello`; the Pi waits for a
matching-version `welcome` before processing commands.

| Direction | Type | Fields |
| --- | --- | --- |
| Pi → server | `hello` | `protocol_version`, `device_id` |
| Server → Pi | `welcome` | `protocol_version` |
| Server → Pi | `error` | `code`, `message` |
| Server → Pi | `capture` | `request_id`, `width`, `height`, `quality` |
| Server → Pi | `record` | `request_id`, `duration_ms` |
| Pi → server | `capture_failed` | `request_id`, `code`, `message` |
| Pi → server | `record_failed` | `request_id`, `code`, `message` |

`RequestId` wraps a `u64` and serializes as a JSON number. All `u64` values are valid
at the protocol layer; the server will allocate IDs starting at 1. A combined photo
and audio request uses the same ID for both commands and responses. The media kind
distinguishes the two successful responses.

Example commands for request 42:

```json
{"type":"capture","request_id":42,"width":1920,"height":1080,"quality":85}
{"type":"record","request_id":42,"duration_ms":30000}
```

The demo recording duration is **30 seconds**. Audio is a complete WAV file
containing mono, 16-bit PCM at 16 kHz. Capture and recording will run concurrently.
Command handlers validate operational parameters, including JPEG quality
`1..=100` and recording duration `1..=30000` milliseconds.

Failures carry the original request ID:

```json
{"type":"capture_failed","request_id":42,"code":"unavailable","message":"Camera unavailable"}
{"type":"record_failed","request_id":42,"code":"timeout","message":"Recording exceeded its deadline"}
```

Media failure codes are `invalid_parameters`, `busy`, `unavailable`, `timeout`,
and `io`. Unrecognized codes deserialize to `MediaErrorCode::Unknown`.
Existing handshake errors retain their separate `ErrorCode` enum.

### Binary media messages (`media.rs`)

For each successful operation, the Pi sends one complete binary WebSocket message.
It contains a **10-byte header** followed by the complete JPEG or WAV file:

| Byte offset | Length | Meaning |
| --- | --- | --- |
| `0` | 1 byte | Binary format version: `1` |
| `1` | 1 byte | Media kind: `1` = JPEG, `2` = WAV |
| `2..10` | 8 bytes | Request ID as a big-endian `u64` |
| `10..` | Remaining bytes | JPEG or WAV payload, including its file header |

Offsets use Rust's half-open range notation: `2..10` means bytes 2 through 9.
The binary format version is a separate `u8` constant from the JSON handshake's
`u16` protocol version; both initially have value 1.

For JPEG request 42, the header in hexadecimal is:

```text
01 01 00 00 00 00 00 00 00 2A
```

The WAV response for the same request has `02` as the second byte. Encode the ID
with `u64::to_be_bytes` and decode it with `u64::from_be_bytes`.
WebSocket message boundaries delimit each complete response, so this header needs
no additional length prefix. If WebSocket fragments are used, the WebSocket library
reassembles them before the media codec runs.

### Limits and codec behavior

- Maximum complete binary message: **8 MiB** (`8 * 1024 * 1024` bytes), including
  the 10-byte header.
- Maximum JPEG payload: **8 MiB minus 10 bytes**.
- Maximum WAV payload: **4 MiB** (`4 * 1024 * 1024` bytes), including the WAV file
  header but excluding the 10-byte media header.
- Empty payloads, truncated headers, unsupported binary versions, and unknown media
  kinds are rejected.
- JSON control messages retain their independent **4 KiB** limit when binary
  support is enabled.

Decode in this order: check the complete message size, require all 10 header bytes,
check the version, decode the kind and request ID, then validate payload length.
Return a borrowed payload slice so decoding does not copy the image or recording.
Encoding validates the payload before allocating a buffer for the header and data.
The codec validates the envelope; JPEG decoding and WAV format validation belong
to the media consumers.

When transport integration is implemented, the server's WebSocket message limit
must allow 8 MiB while text messages still pass through the 4 KiB JSON decoder.
Media is accepted only after a successful handshake and matched to pending work
by request ID and media kind.

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

Paste a valid hello and press Enter:

```json
{"type":"hello","protocol_version":1,"device_id":"camera-01"}
```

Expect `{"type":"welcome","protocol_version":1}` and a "Camera unit online"
status on the display connection. Disconnect and expect "Camera unit offline".
A replacement connection takes ownership; cleanup of the old connection must not
publish a false offline status. Plain `hello` is invalid JSON and receives an error
followed by closure. Abruptly terminating a client can produce a connection-reset
warning before the disconnection log.

These checks validate transport through a command-line client; they do not verify HudBridge, Bluetooth, or physical glasses. Normal sends do not exercise the stalled-send timeout path.

## Development checks

Run from this directory:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Automated tests cover protocol serialization, token validation, broadcasts,
device handshakes, and connection ownership. Real-time heartbeat tests are ignored
by default and can be run with `cargo test -p hudagents-server -- --ignored`.
During the media protocol checkpoint, check the shared crate independently with
`cargo test -p hudagents-proto --offline`.

## Current scope

This is a trusted-network development server. `/say` and `/device` currently have no authentication, and the server does not configure TLS. Only `/phone` checks the shared token.

Device traffic is logged as metadata. The Pi reconnects with growing waits from
1 second to 30 seconds, resetting after a successful welcome. Image/audio capture
and transfer, local model orchestration, and delivery acknowledgements remain to
be implemented.
