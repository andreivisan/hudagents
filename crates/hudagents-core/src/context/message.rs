use super::ids::AgentId;
use crate::agent::{AgentInput, AgentOutput};

#[derive(Clone, Debug)]
pub enum Sender {
    User,
    Agent(AgentId),
}

#[derive(Clone, Debug)]
pub struct AgentMessage {
    pub from: Sender,
    pub text: String,
}

pub enum MessagePayload {
    Text(String),
    Input(AgentInput),
    Output(AgentOutput),
}
