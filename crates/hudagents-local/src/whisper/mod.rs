use std::{
    fmt::{self, Display}, 
    error::Error,
    path::Path,
};
use whisper_rs::{WhisperContext, WhisperContextParameters};

#[derive(Debug)]
pub enum HAWhisperError {
    ModelNotFound(String),
    ModelInitFailed(String),
}

impl Display for HAWhisperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAWhisperError::ModelNotFound(path) => write!(f, 
                "Model not found at: {}\n\
                Use `hudagents-tools sysinfo` to get the recommended model for your system, or\n\
                Use `hudagents-tools download --model <model> --path <path>` to download the model"
                , path
            ), 
            HAWhisperError::ModelInitFailed(msg) => write!(f, "Failed to initialize Whisper context: {}", msg),
        }
    }
}

impl Error for HAWhisperError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

pub struct HALocalWhisper {
    whisper_ctx: WhisperContext,
}

impl HALocalWhisper {
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, HAWhisperError> {
        let path = model_path.as_ref();
        if !path.exists() {
            return Err(HAWhisperError::ModelNotFound(path.display().to_string()));
        }
        let whisper_ctx = WhisperContext::new_with_params(
            path.to_string_lossy().as_ref(), 
            WhisperContextParameters::default()
        ).map_err(|e| HAWhisperError::ModelInitFailed(format!("Error loading model at {:?}: {:?}", path, e)))?;
        
        Ok(Self { whisper_ctx })
    }
}