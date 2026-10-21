use std::{process::Stdio, time::Duration};

use hudagents_proto::media::{MAX_BINARY_MESSAGE_BYTES, MEDIA_HEADER_BYTES};
use tokio::io::AsyncReadExt;

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
        .arg("- /dev/shm/hudagents-camera-check.jpg")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("Some message chat will invent");
    match child.stdout.take() {
        Some(stdout) => {
            let jpeg_bytes = stdout.take(MAX_JPEG_BYTES);
            let mut buffer: Vec<u8> = Vec::new();
            jpeg_bytes.read_to_end(&mut buffer);
            if buffer.len() > MAX_JPEG_BYTES {
                return Err(CaptureError::TooLarge {
                    limit: MAX_JPEG_BYTES,
                });
            }
            match child.wait().await {
                Ok(data) => if data.success() {},
            }
        }
        None => {
            match tokio::time::timeout(CLEANUP_TIMEOUT, child.kill()).await {};
            return Err(CaptureError::MissingStdout);
        }
    }
}
