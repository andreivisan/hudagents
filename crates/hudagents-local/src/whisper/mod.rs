use std::{
    error::Error,
    fmt::{self, Debug, Display},
    path::Path,
};
use whisper_rs::{WhisperContext, WhisperContextParameters};

#[derive(Debug)]
pub enum HAWhisperError {
    ModelNotFound(String),
    InvalidModelName(String),
    ModelInitFailed(String),
    TranscriptionFailed(String),
    MissingDependency(String),
    DecodeFailed(String),
    HttpRequestFailed(reqwest::Error),
    HttpStatus(reqwest::StatusCode),
    IOError(std::io::Error),
}

impl Display for HAWhisperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HAWhisperError::ModelNotFound(path) => write!(
                f,
                "Model not found at: {}\n\
                Use `hudagents-tools sysinfo` to get the recommended model for your system, or\n\
                Use `hudagents-tools download --model <model> --path <path>` to download the model",
                path
            ),
            HAWhisperError::InvalidModelName(model) => write!(f, "Invalid model name: {}", model),
            HAWhisperError::ModelInitFailed(msg) => {
                write!(f, "Failed to initialize Whisper context: {}", msg)
            }
            HAWhisperError::TranscriptionFailed(msg) => {
                write!(f, "Transcription failed: {}", msg)
            }
            HAWhisperError::MissingDependency(dep) => {
                write!(f, "Missing dependency: {}. Please install it.", dep)
            }
            HAWhisperError::DecodeFailed(msg) => {
                write!(f, "ffmpeg failed to decode input: {}", msg)
            }
            HAWhisperError::HttpRequestFailed(msg) => write!(f, "HTTP request failed: {}", msg),
            HAWhisperError::HttpStatus(status) => write!(f, "HTTP status: {}", status.as_u16()),
            HAWhisperError::IOError(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl From<std::io::Error> for HAWhisperError {
    fn from(e: std::io::Error) -> Self {
        HAWhisperError::IOError(e)
    }
}

impl Error for HAWhisperError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

pub struct HALocalWhisper {
    pub whisper_ctx: WhisperContext,
}

impl Debug for HALocalWhisper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let model_type = self
            .whisper_ctx
            .model_type_readable_str_lossy()
            .unwrap_or("unknown".into());

        f.debug_struct("HALocalWhisper")
            // Create a custom formatted string for the context field
            .field(
                "whisper_ctx",
                &format_args!(
                    "WhisperContext {{ model: {:?}, multilingual: {}, vocab: {} }}",
                    model_type,
                    self.whisper_ctx.is_multilingual(),
                    self.whisper_ctx.n_vocab()
                ),
            )
            .finish()
    }
}

impl HALocalWhisper {
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, HAWhisperError> {
        let path = model_path.as_ref();
        if !path.exists() {
            return Err(HAWhisperError::ModelNotFound(path.display().to_string()));
        }
        let whisper_ctx = WhisperContext::new_with_params(
            path.to_string_lossy().as_ref(),
            WhisperContextParameters::default(),
        )
        .map_err(|e| {
            HAWhisperError::ModelInitFailed(format!("Error loading model at {:?}: {:?}", path, e))
        })?;

        Ok(Self { whisper_ctx })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, remove_file};

    #[test]
    fn test_new_fails_when_model_not_found() {
        let bad_path = "bad/path/to/model.bin";
        let result = HALocalWhisper::new(bad_path);

        assert!(result.is_err());

        match result {
            Err(HAWhisperError::ModelNotFound(path)) => assert_eq!(path, bad_path),
            _ => panic!("Expected ModelNotFound error, got {:?}", result),
        }
    }

    #[test]
    fn test_new_fails_when_model_init_failed() {
        let dummy = "dummy_model.bin";
        {
            File::create(dummy).unwrap();
        }
        let result = HALocalWhisper::new(dummy);

        assert!(result.is_err());

        match result {
            Err(HAWhisperError::ModelInitFailed(msg)) => assert!(msg.contains(dummy)),
            _ => panic!("Expected ModelInitFailed error, got {:?}", result),
        }

        remove_file(dummy).unwrap();
    }
}
