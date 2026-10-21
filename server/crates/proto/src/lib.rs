use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod device;
pub mod media;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Text,
    Status,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeMessage {
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub seq: u64,
    pub id: uuid::Uuid,
    pub body: String,
}

impl BridgeMessage {
    pub fn text(seq: u64, body: String) -> Self {
        Self {
            msg_type: MessageType::Text,
            seq,
            id: Uuid::new_v4(),
            body,
        }
    }

    pub fn status(seq: u64, body: String) -> Self {
        Self {
            msg_type: MessageType::Status,
            seq,
            id: Uuid::new_v4(),
            body,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename = "telemetry")]
pub struct BridgeTelemetry {
    seq: u64,
    id: Uuid,
    temp_c: f64,
}

impl BridgeTelemetry {
    pub fn new(seq: u64, temp_c: f64) -> Self {
        BridgeTelemetry {
            seq,
            id: Uuid::new_v4(),
            temp_c,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_serializes_the_bridge_contract_without_a_body() {
        let message = BridgeTelemetry::new(42, 53.25);
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "telemetry",
                "seq": 42,
                "id": message.id.to_string(),
                "temp_c": 53.25,
            })
        );
        assert_eq!(
            Uuid::parse_str(json["id"].as_str().unwrap()).unwrap(),
            message.id
        );
        assert_eq!(message.id.get_version_num(), 4);
    }

    #[test]
    fn telemetry_gives_repeated_readings_fresh_ids() {
        let first = BridgeTelemetry::new(1, 53.25);
        let second = BridgeTelemetry::new(2, 53.25);
        assert_ne!(first.id, second.id);
        assert!(!first.id.is_nil());
        assert!(!second.id.is_nil());
    }

    #[test]
    fn test_serialize_message() {
        let message = BridgeMessage::text(1, "message body".to_owned());
        let serialized =
            serde_json::to_value(&message).expect("BridgeMessage should serialize to JSON");
        assert_eq!(serialized["type"], "text");
        assert_eq!(serialized["seq"], 1);
        assert_eq!(serialized["body"], "message body");
        assert_eq!(serialized["id"], message.id.to_string());
        assert!(serialized.get("msg_type").is_none());
    }
}
