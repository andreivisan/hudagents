pub mod speech_to_text;
pub use hudagents_local::whisper::HAWhisperError;
use std::fmt::{self, Debug, Display};

#[derive(Debug)]
pub enum HAAgentError {
    AgentInputError(String),
    SpeechToTextError(HAWhisperError),
}

impl Display for HAAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAAgentError::AgentInputError(msg) => write!(f, "Agent Input Error: {}", msg),
            HAAgentError::SpeechToTextError(msg) => {
                write!(f, "Audio transcription failed: {}", msg)
            }
        }
    }
}

impl From<HAWhisperError> for HAAgentError {
    fn from(e: HAWhisperError) -> Self {
        HAAgentError::SpeechToTextError(e)
    }
}

pub enum AgentInput {
    // TODO: Add more Audio variants AudioM4a, AudioPcm, etc.
    Audio(Vec<u8>),
    Image(Vec<u8>),
    Text(String),
}

pub enum AgentOutput {
    AudioTranscription(String),
    ImageInterpretation(String),
    FinalAnswer(String),
}

pub trait Agent {
    fn id(&self) -> &str;
    fn call(&self, agent_input: AgentInput) -> Result<AgentOutput, HAAgentError>;
    fn describe(&self) -> String;
}
