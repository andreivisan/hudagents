use super::blob::BlobRef;
use super::ids::RunId;
use super::runtime::Control;
use crate::graph::NodeId;

#[derive(Clone, Debug)]
pub enum Sender {
    User,
    Node(NodeId),
}

#[derive(Clone, Debug)]
pub enum MessagePayload {
    Text(String),
    Audio(BlobRef),
    Image(BlobRef),
    Transcription(String),
    VisionCaption(String),
    FinalAnswer(String),
    Control(Control),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct AgentMessage {
    pub run: RunId,
    pub from: Sender,
    pub payload: MessagePayload,
}
