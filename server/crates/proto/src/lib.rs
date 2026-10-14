use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

#[cfg(test)]
mod tests {
    use super::*;

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
