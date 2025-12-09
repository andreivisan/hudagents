pub use hudagents_local::whisper::{HALocalWhisper, HAWhisperError};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};
use whisper_rs::{FullParams, SamplingStrategy};

pub type WhisperResult<T> = std::result::Result<T, HAWhisperError>;

fn ensure_ffmpeg_installed() -> Result<(), HAWhisperError> {
    match Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        _ => Err(HAWhisperError::MissingDependency(
            "ffmpeg is not installed or not found in PATH".to_string(),
        )),
    }
}

// TODO: Use Thread Pool for ffmpeg decoding to improve performance on multiple
fn decode_m4a_to_f32(input: &[u8]) -> WhisperResult<Vec<f32>> {
    ensure_ffmpeg_installed()?;
    let mut child = Command::new("ffmpeg")
        .args([
            "-i", "pipe:0", // stdin
            "-f", "s16le", "-ac", "1", "-ar", "16000", "pipe:1", // stdout
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    child.stdin.as_mut().unwrap().write_all(input)?;
    drop(child.stdin.take());

    let scale = 1.0f32 / 32768.0;
    let mut pcm_f32 = Vec::<f32>::new();
    let mut buf = [0u8; 4096];
    let mut stdout = child.stdout.take().unwrap();

    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for chunk in buf[..n].chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            pcm_f32.push(sample as f32 * scale);
        }
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(HAWhisperError::DecodeFailed(
            "ffmpeg failed to decode input".to_string(),
        ));
    }

    Ok(pcm_f32)
}

pub fn transcribe(model_path: &str, input: &[u8]) -> WhisperResult<String> {
    let whisper_ctx = match HALocalWhisper::new(model_path) {
        Ok(ctx) => ctx.whisper_ctx,
        Err(e) => return Err(e),
    };
    let mut state = whisper_ctx
        .create_state()
        .map_err(|e| HAWhisperError::ModelInitFailed(format!("Error creating state: {:?}", e)))?;

    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(n_threads as i32);
    params.set_translate(false);
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    let pcm_samples = decode_m4a_to_f32(input)?;

    state.full(params, &pcm_samples).map_err(|e| {
        HAWhisperError::ModelInitFailed(format!("Error during transcription: {:?}", e))
    })?;
    let mut transcript = String::new();
    for segment in state.as_iter() {
        let segment_text = segment.to_str_lossy().map_err(|e| {
            HAWhisperError::TranscriptionFailed(format!(
                "Error retrieving segment text for segment {}: {:?}",
                segment, e
            ))
        })?;
        transcript.push_str(&segment_text);
        transcript.push(' ');
    }
    Ok(transcript.trim().to_string())
}
