pub use hudagents_local::whisper::{HALocalWhisper, HAWhisperError};
use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
};
use whisper_rs::{FullParams, SamplingStrategy};

pub type WhisperResult<T> = std::result::Result<T, HAWhisperError>;

fn decode_m4a_from_bytes(input: &[u8]) -> io::Result<Vec<i16>> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-i", "pipe:0", // stdin
            "-f", "s16le", "-ac", "1", "-ar", "16000", "pipe:1", // stdout
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin - piped");
        stdin.write_all(input)?;
    }
    let mut raw_output = Vec::new();
    child
        .stdout
        .take()
        .expect("Failed to open stdout - piped")
        .read_to_end(&mut raw_output)?;
    let samples = raw_output
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(samples)
}

fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    let scale = 1.0f32 / 32768.0;
    let mut out = Vec::with_capacity(samples.len());
    for &s in samples {
        out.push(s as f32 * scale);
    }
    out
}

pub fn transcribe(model_path: &str, input: &[u8]) -> WhisperResult<String> {
    let whisper_ctx = match HALocalWhisper::new(model_path) {
        Ok(ctx) => ctx.whisper_ctx,
        Err(e) => return Err(e),
    };
    let mut state = whisper_ctx
        .create_state()
        .map_err(|e| HAWhisperError::ModelInitFailed(format!("Error creating state: {:?}", e)))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(1);
    params.set_translate(false);
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    let pcm_samples = match decode_m4a_from_bytes(input) {
        Ok(samples) => i16_to_f32(&samples),
        Err(e) => {
            return Err(HAWhisperError::IOError(e));
        }
    };

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
