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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, path::Path};

    #[test]
    fn test_decode_m4a_to_f32() {
        let input_data = include_bytes!("test_data/good-m4a.m4a");
        let result = decode_m4a_to_f32(input_data);
        assert!(result.is_ok());
        let pcm_f32 = result.unwrap();
        assert!(!pcm_f32.is_empty());
    }

    #[test]
    fn test_decode_m4a_to_f32_corrupt_input() {
        let input_data = include_bytes!("test_data/bad-m4a.m4a");
        let result = decode_m4a_to_f32(input_data);
        assert!(result.is_err());
    }

    //TODO: set a flag or ignore for when a CI environment is used
    #[test]
    fn test_transcribe() {
        let model_dir = env::var("HA_WHISPER_PATH").expect("HA_WHISPER_PATH must be set for tests");
        let model_path = Path::new(&model_dir).join("medium.en.bin");
        let model_path = model_path
            .to_str()
            .expect("Model path should be valid UTF-8");
        let input_data = include_bytes!("test_data/good-m4a.m4a");
        let result = transcribe(model_path, input_data);
        assert!(result.is_ok());
        let transcript = result.unwrap();
        assert!(!transcript.is_empty());
        println!("Transcript: {}", transcript);
    }
}
