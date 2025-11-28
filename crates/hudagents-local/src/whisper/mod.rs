use std::{
    fmt::{self, Display}, 
    error::Error,
};

#[derive(Debug)]
pub enum HAWhisperError {
    ModelNotFound(String),
}

impl Display for HAWhisperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAWhisperError::ModelNotFound(path) => write(f, "Model not found at: {}", path), //TODO: add how to use the tools CLI to download the model
        }
    }
}

impl Error for HAWhisperError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            HAWhisperError::ModelNotFound(_) => None,
        }
    }
}