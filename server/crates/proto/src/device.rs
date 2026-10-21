use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{MAX_CONTROL_MESSAGE_BYTES, PROTOCOL_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct DeviceId(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidDeviceId {
    #[error("device id must not be empty")]
    Empty,

    #[error("device id is {len} bytes; maximum is {}", DeviceId::MAX_LEN)]
    TooLong { len: usize },

    #[error("device id contains invalid character {ch:?} at byte {index}")]
    InvalidChar { ch: char, index: usize },

    #[error("device id can't start or end with '-'")]
    EdgeHyphen,
}

impl DeviceId {
    pub const MAX_LEN: usize = 64;

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DeviceId {
    type Error = InvalidDeviceId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(InvalidDeviceId::Empty);
        }
        if value.len() > Self::MAX_LEN {
            return Err(InvalidDeviceId::TooLong { len: value.len() });
        }
        if let Some((index, ch)) = value
            .char_indices()
            .find(|&(_, ch)| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'))
        {
            return Err(InvalidDeviceId::InvalidChar { ch, index });
        }
        if value.starts_with('-') || value.ends_with('-') {
            return Err(InvalidDeviceId::EdgeHyphen);
        }
        Ok(DeviceId(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(u64);

impl RequestId {
    pub fn new(value: u64) -> Self {
        RequestId(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceMessage {
    Hello {
        protocol_version: u16,
        device_id: DeviceId,
    },
    CaptureFailed {
        request_id: RequestId,
        code: MediaErrorCode,
        message: String,
    },
    RecordFailed {
        request_id: RequestId,
        code: MediaErrorCode,
        message: String,
    },
    Telemetry {
        temp_millicelsius: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Capture {
        request_id: RequestId,
        width: u16,
        height: u16,
        quality: u8,
    },
    Record {
        request_id: RequestId,
        duration_ms: u32,
    },
    Welcome {
        protocol_version: u16,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedVersion,
    InvalidMessage,
    UnexpectedMessage,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaErrorCode {
    InvalidParameters,
    Busy,
    Unavailable,
    Timeout,
    Io,
    #[serde(other)]
    Unknown,
}

impl DeviceMessage {
    pub fn hello(device_id: DeviceId) -> Self {
        Self::Hello {
            protocol_version: PROTOCOL_VERSION,
            device_id,
        }
    }

    pub fn from_json(text: &str) -> Result<Self, ProtocolError> {
        decode(text)
    }

    pub fn to_json(&self) -> Result<String, ProtocolError> {
        encode(self)
    }
}

impl ServerMsg {
    pub fn welcome() -> Self {
        Self::Welcome {
            protocol_version: PROTOCOL_VERSION,
        }
    }

    pub fn error(code: ErrorCode, message: String) -> Self {
        Self::Error { code, message }
    }

    pub fn from_json(text: &str) -> Result<Self, ProtocolError> {
        decode(text)
    }

    pub fn to_json(&self) -> Result<String, ProtocolError> {
        encode(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Message is {len} bytes; the limit is {MAX_CONTROL_MESSAGE_BYTES}")]
    TooLarge { len: usize },
    #[error("invalid message: {0}")]
    Invalid(#[from] serde_json::Error),
}

fn decode<T: DeserializeOwned>(text: &str) -> Result<T, ProtocolError> {
    if text.len() > MAX_CONTROL_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { len: text.len() });
    }
    let decoded = serde_json::from_str(text)?;
    Ok(decoded)
}

fn encode<T: Serialize>(message: &T) -> Result<String, ProtocolError> {
    let serialized = serde_json::to_string(message)?;
    if serialized.len() > MAX_CONTROL_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge {
            len: serialized.len(),
        });
    }
    Ok(serialized)
}

#[cfg(test)]
mod tests {
    use super::{
        DeviceId, DeviceMessage, ErrorCode, InvalidDeviceId, MAX_CONTROL_MESSAGE_BYTES,
        MediaErrorCode, ProtocolError, RequestId, ServerMsg,
    };

    const HELLO: &str = r#"{"type":"hello","protocol_version":1,"device_id":"camera-01"}"#;
    const WELCOME: &str = r#"{"type":"welcome","protocol_version":1}"#;
    const CAPTURE: &str =
        r#"{"type":"capture","request_id":42,"width":1920,"height":1080,"quality":85}"#;
    const RECORD: &str = r#"{"type":"record","request_id":42,"duration_ms":30000}"#;
    const CAPTURE_FAILED: &str = r#"{"type":"capture_failed","request_id":42,"code":"unavailable","message":"Camera unavailable"}"#;
    const RECORD_FAILED: &str = r#"{"type":"record_failed","request_id":42,"code":"timeout","message":"Recording exceeded its deadline"}"#;

    #[test]
    fn telemetry_matches_wire_format_and_round_trips() {
        let json = r#"{"type":"telemetry","temp_millicelsius":53250}"#;
        let message = DeviceMessage::Telemetry {
            temp_millicelsius: 53_250,
        };
        assert_eq!(message.to_json().unwrap(), json);
        assert_eq!(DeviceMessage::from_json(json).unwrap(), message);
        assert!(ServerMsg::from_json(json).is_err());
    }

    #[test]
    fn telemetry_preserves_signed_integers_for_handler_validation() {
        // Plausible temperature limits belong to the reader and server handler;
        // the wire type itself represents every i32 value without rounding.
        for value in [
            i32::MIN,
            -40_001,
            -40_000,
            -1,
            0,
            150_000,
            150_001,
            i32::MAX,
        ] {
            let message = DeviceMessage::Telemetry {
                temp_millicelsius: value,
            };
            assert_eq!(
                DeviceMessage::from_json(&message.to_json().unwrap()).unwrap(),
                message
            );
        }
    }

    #[test]
    fn telemetry_rejects_missing_mistyped_and_overflowing_temperature() {
        assert!(DeviceMessage::from_json(r#"{"type":"telemetry"}"#).is_err());
        for value in [
            "null",
            "true",
            r#""53250""#,
            "53.25",
            "53250.0",
            "[]",
            "{}",
            "2147483648",
            "-2147483649",
        ] {
            let json = format!(r#"{{"type":"telemetry","temp_millicelsius":{value}}}"#);
            assert!(DeviceMessage::from_json(&json).is_err(), "accepted {json}");
        }
    }

    #[test]
    fn telemetry_ignores_extra_fields() {
        let json = r#"{"type":"telemetry","temp_millicelsius":53250,"firmware":"0.2"}"#;
        assert_eq!(
            DeviceMessage::from_json(json).unwrap(),
            DeviceMessage::Telemetry {
                temp_millicelsius: 53_250,
            }
        );
    }

    #[test]
    fn telemetry_obeys_the_control_message_size_limit() {
        let json = r#"{"type":"telemetry","temp_millicelsius":53250}"#;
        let at_limit = format!(
            "{json}{}",
            " ".repeat(MAX_CONTROL_MESSAGE_BYTES - json.len())
        );
        assert!(DeviceMessage::from_json(&at_limit).is_ok());
        let oversized = format!("{at_limit} ");
        assert!(matches!(DeviceMessage::from_json(&oversized),
            Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1
        ));
    }

    #[test]
    fn request_id_is_a_json_integer_and_preserves_the_full_u64_range() {
        for value in [0, 1, 42, (1_u64 << 53) + 1, u64::MAX] {
            let id = RequestId::new(value);
            assert_eq!(id.get(), value);
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, value.to_string());
            assert_eq!(serde_json::from_str::<RequestId>(&json).unwrap(), id);
        }
    }

    #[test]
    fn request_id_rejects_non_u64_json_values() {
        for input in [
            "-1",
            "18446744073709551616",
            "1.5",
            "1.0",
            r#""42""#,
            "null",
            "true",
            "[]",
            "{}",
        ] {
            assert!(
                serde_json::from_str::<RequestId>(input).is_err(),
                "accepted invalid request id: {input}"
            );
        }
    }

    #[test]
    fn capture_and_record_match_wire_format_and_round_trip() {
        let examples = [
            (
                ServerMsg::Capture {
                    request_id: RequestId::new(42),
                    width: 1920,
                    height: 1080,
                    quality: 85,
                },
                CAPTURE,
            ),
            (
                ServerMsg::Record {
                    request_id: RequestId::new(42),
                    duration_ms: 30_000,
                },
                RECORD,
            ),
        ];
        for (message, expected) in examples {
            let json = message.to_json().unwrap();
            // Compare JSON structure: object field ordering is not part of the contract.
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&json).unwrap(),
                serde_json::from_str::<serde_json::Value>(expected).unwrap()
            );
            assert_eq!(ServerMsg::from_json(expected).unwrap(), message);
            assert_eq!(ServerMsg::from_json(&json).unwrap(), message);
        }
    }

    #[test]
    fn media_failures_match_wire_format_and_round_trip() {
        let examples = [
            (
                DeviceMessage::CaptureFailed {
                    request_id: RequestId::new(42),
                    code: MediaErrorCode::Unavailable,
                    message: "Camera unavailable".into(),
                },
                CAPTURE_FAILED,
            ),
            (
                DeviceMessage::RecordFailed {
                    request_id: RequestId::new(42),
                    code: MediaErrorCode::Timeout,
                    message: "Recording exceeded its deadline".into(),
                },
                RECORD_FAILED,
            ),
        ];
        for (message, expected) in examples {
            let json = message.to_json().unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&json).unwrap(),
                serde_json::from_str::<serde_json::Value>(expected).unwrap()
            );
            assert_eq!(DeviceMessage::from_json(expected).unwrap(), message);
            assert_eq!(DeviceMessage::from_json(&json).unwrap(), message);
        }
    }

    #[test]
    fn media_error_codes_have_stable_snake_case_names() {
        for (code, wire) in [
            (MediaErrorCode::InvalidParameters, "invalid_parameters"),
            (MediaErrorCode::Busy, "busy"),
            (MediaErrorCode::Unavailable, "unavailable"),
            (MediaErrorCode::Timeout, "timeout"),
            (MediaErrorCode::Io, "io"),
            (MediaErrorCode::Unknown, "unknown"),
        ] {
            let json = format!("\"{wire}\"");
            assert_eq!(serde_json::to_string(&code).unwrap(), json);
            assert_eq!(serde_json::from_str::<MediaErrorCode>(&json).unwrap(), code);
        }
    }

    #[test]
    fn unknown_media_error_codes_preserve_failure_and_message() {
        let message = "Microphone \"busy\"\nTry again — later.";
        for kind in ["capture_failed", "record_failed"] {
            let json = serde_json::json!({
                "type": kind,
                "request_id": 42,
                "code": "new_firmware_error",
                "message": message,
            })
            .to_string();
            let expected = if kind == "capture_failed" {
                DeviceMessage::CaptureFailed {
                    request_id: RequestId::new(42),
                    code: MediaErrorCode::Unknown,
                    message: message.into(),
                }
            } else {
                DeviceMessage::RecordFailed {
                    request_id: RequestId::new(42),
                    code: MediaErrorCode::Unknown,
                    message: message.into(),
                }
            };
            let decoded = DeviceMessage::from_json(&json).unwrap();
            assert_eq!(decoded, expected);
            assert_eq!(
                DeviceMessage::from_json(&decoded.to_json().unwrap()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn media_messages_reject_the_wrong_direction() {
        for input in [CAPTURE, RECORD] {
            assert!(DeviceMessage::from_json(input).is_err());
        }
        for input in [CAPTURE_FAILED, RECORD_FAILED] {
            assert!(ServerMsg::from_json(input).is_err());
        }
    }

    #[test]
    fn media_commands_reject_missing_and_null_fields() {
        for input in [CAPTURE, RECORD] {
            let original: serde_json::Value = serde_json::from_str(input).unwrap();
            for field in original.as_object().unwrap().keys() {
                let mut missing = original.clone();
                missing.as_object_mut().unwrap().remove(field);
                assert!(
                    matches!(
                        ServerMsg::from_json(&missing.to_string()),
                        Err(ProtocolError::Invalid(_))
                    ),
                    "accepted missing {field}: {missing}"
                );
                let mut null = original.clone();
                null[field] = serde_json::Value::Null;
                assert!(
                    matches!(
                        ServerMsg::from_json(&null.to_string()),
                        Err(ProtocolError::Invalid(_))
                    ),
                    "accepted null {field}: {null}"
                );
            }
        }
    }

    #[test]
    fn media_commands_reject_wrong_types_and_integer_overflow() {
        use serde_json::json;

        for (input, field, value) in [
            (CAPTURE, "request_id", json!("42")),
            (CAPTURE, "request_id", json!(-1)),
            (CAPTURE, "width", json!("1920")),
            (CAPTURE, "width", json!(-1)),
            (CAPTURE, "width", json!(65_536)),
            (CAPTURE, "height", json!(1080.5)),
            (CAPTURE, "height", json!(65_536)),
            (CAPTURE, "quality", json!("85")),
            (CAPTURE, "quality", json!(-1)),
            (CAPTURE, "quality", json!(256)),
            (RECORD, "request_id", json!(42.5)),
            (RECORD, "duration_ms", json!("30000")),
            (RECORD, "duration_ms", json!(-1)),
            (RECORD, "duration_ms", json!(30_000.5)),
            (RECORD, "duration_ms", json!(u64::from(u32::MAX) + 1)),
        ] {
            let mut message: serde_json::Value = serde_json::from_str(input).unwrap();
            message[field] = value;
            assert!(
                matches!(
                    ServerMsg::from_json(&message.to_string()),
                    Err(ProtocolError::Invalid(_))
                ),
                "accepted invalid command: {message}"
            );
        }
    }

    #[test]
    fn media_failures_reject_missing_null_and_mistyped_fields() {
        for input in [CAPTURE_FAILED, RECORD_FAILED] {
            let original: serde_json::Value = serde_json::from_str(input).unwrap();
            for field in original.as_object().unwrap().keys() {
                let mut missing = original.clone();
                missing.as_object_mut().unwrap().remove(field);
                assert!(
                    matches!(
                        DeviceMessage::from_json(&missing.to_string()),
                        Err(ProtocolError::Invalid(_))
                    ),
                    "accepted missing {field}: {missing}"
                );
                for invalid in [serde_json::Value::Null, serde_json::json!(true)] {
                    let mut message = original.clone();
                    message[field] = invalid;
                    assert!(
                        matches!(
                            DeviceMessage::from_json(&message.to_string()),
                            Err(ProtocolError::Invalid(_))
                        ),
                        "accepted invalid {field}: {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn media_messages_accept_additional_fields() {
        for input in [CAPTURE, RECORD, CAPTURE_FAILED, RECORD_FAILED] {
            let mut message: serde_json::Value = serde_json::from_str(input).unwrap();
            message["future_option"] = serde_json::json!({"enabled": true});
            let extended = message.to_string();
            if matches!(input, CAPTURE | RECORD) {
                assert_eq!(
                    ServerMsg::from_json(&extended).unwrap(),
                    ServerMsg::from_json(input).unwrap()
                );
            } else {
                assert_eq!(
                    DeviceMessage::from_json(&extended).unwrap(),
                    DeviceMessage::from_json(input).unwrap()
                );
            }
        }
    }

    #[test]
    fn media_commands_keep_the_four_kib_control_limit() {
        for input in [CAPTURE, RECORD] {
            let mut at_limit = format!(
                "{input}{}",
                " ".repeat(MAX_CONTROL_MESSAGE_BYTES - input.len())
            );
            assert_eq!(
                ServerMsg::from_json(&at_limit).unwrap(),
                ServerMsg::from_json(input).unwrap()
            );
            at_limit.push(' ');
            assert!(matches!(
                ServerMsg::from_json(&at_limit),
                Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1
            ));
        }
    }

    #[test]
    fn media_failure_encoding_and_decoding_enforce_the_control_limit() {
        for input in [CAPTURE_FAILED, RECORD_FAILED] {
            let mut value: serde_json::Value = serde_json::from_str(input).unwrap();
            value["message"] = serde_json::json!("");
            let overhead = value.to_string().len();
            value["message"] = serde_json::json!("a".repeat(MAX_CONTROL_MESSAGE_BYTES - overhead));
            let json = value.to_string();
            assert_eq!(json.len(), MAX_CONTROL_MESSAGE_BYTES);
            let message = DeviceMessage::from_json(&json).unwrap();
            assert_eq!(message.to_json().unwrap().len(), MAX_CONTROL_MESSAGE_BYTES);

            match value.get_mut("message").unwrap() {
                serde_json::Value::String(text) => text.push('a'),
                _ => unreachable!("test message must be a string"),
            }
            let oversized_json = value.to_string();
            let oversized_message: DeviceMessage = serde_json::from_str(&oversized_json).unwrap();
            assert!(matches!(
                oversized_message.to_json(),
                Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1
            ));
            assert!(matches!(
                DeviceMessage::from_json(&oversized_json),
                Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1
            ));
        }
    }

    #[test]
    fn hello_matches_wire_format_and_round_trips() {
        let id = DeviceId::try_from("camera-01".to_owned()).unwrap();
        let message = DeviceMessage::hello(id);
        let json = message.to_json().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::json!({
                "type": "hello",
                "protocol_version": 1,
                "device_id": "camera-01"
            })
        );
        assert_eq!(DeviceMessage::from_json(HELLO).unwrap(), message);
        assert_eq!(DeviceMessage::from_json(&json).unwrap(), message);
    }

    #[test]
    fn server_messages_match_wire_format_and_round_trip() {
        let examples = [
            (
                ServerMsg::welcome(),
                serde_json::json!({"type": "welcome", "protocol_version": 1}),
            ),
            (
                ServerMsg::error(
                    ErrorCode::UnsupportedVersion,
                    "Expected protocol version 1, got 2".into(),
                ),
                serde_json::json!({
                    "type": "error",
                    "code": "unsupported_version",
                    "message": "Expected protocol version 1, got 2"
                }),
            ),
        ];
        for (message, expected) in examples {
            let json = message.to_json().unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&json).unwrap(),
                expected
            );
            assert_eq!(ServerMsg::from_json(&json).unwrap(), message);
        }
    }

    #[test]
    fn hello_decoding_rejects_invalid_json_fields_and_ids() {
        for input in [
            "garbage",
            "null",
            r#"{"type":"hello","device_id":"camera-01"}"#,
            r#"{"type":"hello","protocol_version":1}"#,
            r#"{"type":"hello","protocol_version":"1","device_id":"camera-01"}"#,
            r#"{"type":"hello","protocol_version":-1,"device_id":"camera-01"}"#,
            r#"{"type":"hello","protocol_version":65536,"device_id":"camera-01"}"#,
            r#"{"type":"hello","protocol_version":1,"device_id":123}"#,
            r#"{"type":"hello","protocol_version":1,"device_id":"Camera-01"}"#,
            r#"{"type":"hello","protocol_version":1,"device_id":""}"#,
            r#"{"type":"hello","protocol_version":1,"device_id":"camera-"}"#,
            r#"{"protocol_version":1,"device_id":"camera-01"}"#,
            r#"{"type":"capture","protocol_version":1,"device_id":"camera-01"}"#,
        ] {
            assert!(
                matches!(
                    DeviceMessage::from_json(input),
                    Err(ProtocolError::Invalid(_))
                ),
                "accepted invalid input: {input}"
            );
        }
    }

    #[test]
    fn server_decoding_rejects_invalid_fields_and_unknown_message_types() {
        for input in [
            r#"{"type":"welcome"}"#,
            r#"{"type":"welcome","protocol_version":"1"}"#,
            r#"{"type":"error","message":"bad request"}"#,
            r#"{"type":"error","code":"invalid_message"}"#,
            r#"{"type":"error","code":42,"message":"bad request"}"#,
            r#"{"type":"error","code":"invalid_message","message":null}"#,
            r#"{"protocol_version":1}"#,
            r#"{"type":"something_new"}"#,
        ] {
            assert!(
                matches!(ServerMsg::from_json(input), Err(ProtocolError::Invalid(_))),
                "accepted invalid input: {input}"
            );
        }
    }

    #[test]
    fn message_directions_do_not_mix() {
        let error = ServerMsg::error(ErrorCode::InvalidMessage, "bad request".into())
            .to_json()
            .unwrap();
        for input in [WELCOME, error.as_str()] {
            assert!(DeviceMessage::from_json(input).is_err());
        }
        assert!(ServerMsg::from_json(HELLO).is_err());
    }

    #[test]
    fn decoding_accepts_additional_fields_for_compatibility() {
        let hello =
            r#"{"type":"hello","protocol_version":1,"device_id":"camera-01","firmware":"0.2"}"#;
        let welcome = r#"{"type":"welcome","protocol_version":1,"server_name":"mac"}"#;
        assert_eq!(
            DeviceMessage::from_json(hello).unwrap(),
            DeviceMessage::from_json(HELLO).unwrap()
        );
        assert_eq!(ServerMsg::from_json(welcome).unwrap(), ServerMsg::welcome());
    }

    #[test]
    fn unknown_error_code_remains_an_error() {
        let input = r#"{"type":"error","code":"rate_limited","message":"slow down"}"#;
        assert_eq!(
            ServerMsg::from_json(input).unwrap(),
            ServerMsg::error(ErrorCode::Unknown, "slow down".into())
        );
    }

    #[test]
    fn decoding_checks_size_before_json_parsing_and_accepts_the_boundary() {
        let hello = format!(
            "{HELLO}{}",
            " ".repeat(MAX_CONTROL_MESSAGE_BYTES - HELLO.len())
        );
        let welcome = format!(
            "{WELCOME}{}",
            " ".repeat(MAX_CONTROL_MESSAGE_BYTES - WELCOME.len())
        );
        assert!(DeviceMessage::from_json(&hello).is_ok());
        assert!(ServerMsg::from_json(&welcome).is_ok());
        assert!(
            matches!(DeviceMessage::from_json(&(hello + " ")), Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1)
        );
        assert!(
            matches!(ServerMsg::from_json(&(welcome + " ")), Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1)
        );

        let oversized_invalid_json = "!".repeat(MAX_CONTROL_MESSAGE_BYTES + 1);
        assert!(matches!(
            DeviceMessage::from_json(&oversized_invalid_json),
            Err(ProtocolError::TooLarge { .. })
        ));
        assert!(matches!(
            ServerMsg::from_json(&oversized_invalid_json),
            Err(ProtocolError::TooLarge { .. })
        ));
    }

    #[test]
    fn encoding_accepts_the_boundary_and_rejects_one_extra_byte() {
        let overhead = ServerMsg::error(ErrorCode::InvalidMessage, String::new())
            .to_json()
            .unwrap()
            .len();
        let message = ServerMsg::error(
            ErrorCode::InvalidMessage,
            "a".repeat(MAX_CONTROL_MESSAGE_BYTES - overhead),
        );
        let json = message.to_json().unwrap();
        assert_eq!(json.len(), MAX_CONTROL_MESSAGE_BYTES);
        assert_eq!(ServerMsg::from_json(&json).unwrap(), message);

        let too_large = ServerMsg::error(
            ErrorCode::InvalidMessage,
            "a".repeat(MAX_CONTROL_MESSAGE_BYTES - overhead + 1),
        );
        assert!(
            matches!(too_large.to_json(), Err(ProtocolError::TooLarge { len }) if len == MAX_CONTROL_MESSAGE_BYTES + 1)
        );
    }

    #[test]
    fn size_limit_counts_utf8_bytes_and_json_escaping() {
        // Both bodies have fewer than 4096 characters, but their JSON exceeds 4096 bytes.
        for body in [
            "é".repeat(MAX_CONTROL_MESSAGE_BYTES / 2),
            "\n".repeat(MAX_CONTROL_MESSAGE_BYTES / 2),
        ] {
            let message = ServerMsg::error(ErrorCode::InvalidMessage, body);
            let json = serde_json::to_string(&message).unwrap();
            assert!(
                matches!(message.to_json(), Err(ProtocolError::TooLarge { len }) if len == json.len())
            );
            assert!(
                matches!(ServerMsg::from_json(&json), Err(ProtocolError::TooLarge { len }) if len == json.len())
            );
        }
    }

    #[test]
    fn accepts_valid_ids_and_preserves_their_text() {
        for input in [
            "camera-01",
            "a",
            "0",
            "cam--01",
            "0123456789",
            "abcdefghijklmnopqrstuvwxyz",
        ] {
            let id = DeviceId::try_from(input.to_owned()).expect("valid device id");
            assert_eq!(id.as_str(), input);
        }
    }

    #[test]
    fn accepts_id_at_the_64_byte_limit() {
        let input = "a".repeat(64);
        let id = DeviceId::try_from(input.clone()).expect("64-byte id should be accepted");
        assert_eq!(id.as_str(), input);
    }

    #[test]
    fn rejects_empty_id() {
        assert_eq!(
            DeviceId::try_from(String::new()),
            Err(InvalidDeviceId::Empty)
        );
    }

    #[test]
    fn rejects_id_one_byte_over_the_limit() {
        assert_eq!(
            DeviceId::try_from("a".repeat(65)),
            Err(InvalidDeviceId::TooLong { len: 65 })
        );
    }

    #[test]
    fn rejects_invalid_characters_with_their_position() {
        for (input, ch, index) in [
            ("Camera", 'C', 0),
            ("cam_01", '_', 3),
            ("cámara", 'á', 1),
            ("cam 01", ' ', 3),
            ("cam.01", '.', 3),
            ("cam\n", '\n', 3),
            ("cam\0", '\0', 3),
            ("cam👓", '👓', 3),
        ] {
            assert_eq!(
                DeviceId::try_from(input.to_owned()),
                Err(InvalidDeviceId::InvalidChar { ch, index }),
                "unexpected validation result for {input:?}"
            );
        }
    }

    #[test]
    fn rejects_leading_or_trailing_hyphens() {
        for input in ["-camera", "camera-", "-camera-", "-", "--"] {
            assert_eq!(
                DeviceId::try_from(input.to_owned()),
                Err(InvalidDeviceId::EdgeHyphen),
                "unexpected validation result for {input:?}"
            );
        }
    }
}
