use std::{process::Stdio, time::Duration};

use hudagents_proto::{
    device::MediaErrorCode,
    media::{MAX_BINARY_MESSAGE_BYTES, MEDIA_HEADER_BYTES},
};
use tokio::{io::AsyncReadExt, process::Child, time::timeout};

pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_JPEG_BYTES: usize = MAX_BINARY_MESSAGE_BYTES - MEDIA_HEADER_BYTES;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error(
        "Unsupported capture dimensions {width}x{height}; width must be 1..=1920 and height must be 1..=1080"
    )]
    InvalidDimensions { width: u16, height: u16 },

    #[error("JPEG quality must be between 1 and 100; received {quality}")]
    InvalidQuality { quality: u8 },

    #[error("Camera capture I/O failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("rpicam-still failed with {status}")]
    ProcessFailed { status: std::process::ExitStatus },

    #[error("Camera capture timed out after 5 seconds")]
    Timeout,

    #[error("Camera JPEG exceeded the {limit}-byte payload limit")]
    TooLarge { limit: usize },

    #[error("Camera capture returned empty or incomplete JPEG data")]
    InvalidJpeg,

    #[error("Camera capture stdout pipe was not created")]
    MissingStdout,
}

impl CaptureError {
    pub fn code(&self) -> MediaErrorCode {
        match self {
            CaptureError::InvalidDimensions { .. } | CaptureError::InvalidQuality { .. } => {
                MediaErrorCode::InvalidParameters
            }
            CaptureError::Timeout => MediaErrorCode::Timeout,
            CaptureError::ProcessFailed { .. } => MediaErrorCode::Unavailable,
            CaptureError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
                MediaErrorCode::Unavailable
            }
            _ => MediaErrorCode::Io,
        }
    }
}

pub fn validate_capture(width: u16, height: u16, quality: u8) -> Result<(), CaptureError> {
    match (width, height, quality) {
        (1..=1920, 1..=1080, 1..=100) => Ok(()),
        (1..=1920, 1..=1080, _) => Err(CaptureError::InvalidQuality { quality }),
        _ => Err(CaptureError::InvalidDimensions { width, height }),
    }
}

pub async fn capture_jpeg(width: u16, height: u16, quality: u8) -> Result<Vec<u8>, CaptureError> {
    validate_capture(width, height, quality)?;
    let mut child = tokio::process::Command::new("rpicam-still")
        .arg("--nopreview")
        .arg("--timeout")
        .arg("1000")
        .arg("--autofocus-on-capture")
        .arg("--encoding")
        .arg("jpg")
        .arg("--width")
        .arg(width.to_string())
        .arg("--height")
        .arg(height.to_string())
        .arg("--quality")
        .arg(quality.to_string())
        .arg("--output")
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            cleanup_child(&mut child).await;
            return Err(CaptureError::MissingStdout);
        }
    };

    // Both reading stdout and waiting for exit share the same five-second budget.
    let timed_result = timeout(CAPTURE_TIMEOUT, async {
        // One extra byte distinguishes an oversized JPEG from one exactly at the limit.
        let mut reader = stdout.take((MAX_JPEG_BYTES + 1) as u64);
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await?;
        if buffer.len() > MAX_JPEG_BYTES {
            return Err(CaptureError::TooLarge {
                limit: MAX_JPEG_BYTES,
            });
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err(CaptureError::ProcessFailed { status });
        }

        // Check the JPEG boundary markers; this is not a full image decoder.
        if buffer.len() < 4
            || !buffer.starts_with(&[0xFF, 0xD8])
            || !buffer.ends_with(&[0xFF, 0xD9])
        {
            return Err(CaptureError::InvalidJpeg);
        }

        Ok(buffer)
    })
    .await;

    let result = match timed_result {
        Ok(result) => result,
        Err(_) => Err(CaptureError::Timeout),
    };
    if result.is_err() {
        cleanup_child(&mut child).await;
    }
    result
}

async fn cleanup_child(child: &mut Child) {
    // A completed wait has already reaped the process.
    if child.id().is_none() {
        return;
    }

    // Tokio's kill also waits for the process. Preserve the original capture error
    // if cleanup fails; kill_on_drop remains enabled when this handle is dropped.
    match timeout(CLEANUP_TIMEOUT, child.kill()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("Failed to stop camera process after capture failure: {error}");
        }
        Err(_) => {
            eprintln!("Camera process cleanup timed out after 1 second");
        }
    }
}
