pub mod speech_to_text;
pub use hudagents_local::whisper::HAWhisperError;
use std::{
    error::Error,
    fmt::{self, Debug, Display},
};

#[derive(Debug)]
pub enum HAAgentError {
    InvalidInput(String),
    Whisper(HAWhisperError),
}

impl Display for HAAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAAgentError::InvalidInput(msg) => write!(f, "agent Input Error: {}", msg),
            HAAgentError::Whisper(msg) => {
                write!(f, "audio transcription failed: {}", msg)
            }
        }
    }
}

impl From<HAWhisperError> for HAAgentError {
    fn from(e: HAWhisperError) -> Self {
        HAAgentError::Whisper(e)
    }
}

impl Error for HAAgentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            HAAgentError::Whisper(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum AgentInput {
    // TODO: Add more Audio variants AudioM4a, AudioPcm, etc.
    Audio(Vec<u8>),
    Image(Vec<u8>),
    Text(String),
}

#[derive(Debug)]
pub enum AgentOutput {
    AudioTranscription(String),
    ImageInterpretation(String),
    FinalAnswer(String),
}

pub trait Agent {
    fn id(&self) -> &str;
    fn call(&self, agent_input: AgentInput) -> Result<AgentOutput, HAAgentError>;
    fn describe(&self) -> String {
        self.id().to_string()
    }
}
