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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceMessage {
    Hello {
        protocol_version: u16,
        device_id: DeviceId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome { protocol_version: u16 },
    Error { code: ErrorCode, message: String },
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
        ProtocolError, ServerMsg,
    };

    const HELLO: &str = r#"{"type":"hello","protocol_version":1,"device_id":"camera-01"}"#;
    const WELCOME: &str = r#"{"type":"welcome","protocol_version":1}"#;

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
