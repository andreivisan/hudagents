pub use crate::agent::{Agent, AgentInput, AgentOutput, HAAgentError};
pub use hudagents_local::whisper::{HALocalWhisper, HAWhisperError};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};
use whisper_rs::{FullParams, SamplingStrategy};

pub type WhisperResult<T> = std::result::Result<T, HAWhisperError>;

pub struct SpeechToTextAgent {
    id: &'static str,
    model_path: String,
}

impl Agent for SpeechToTextAgent {
    fn id(&self) -> &str {
        self.id
    }

    fn call(&self, agent_input: AgentInput) -> Result<AgentOutput, HAAgentError> {
        match agent_input {
            AgentInput::Audio(bytes) => {
                let text = transcribe(&self.model_path, &bytes)?;
                Ok(AgentOutput::AudioTranscription(text))
            }
            _ => Err(HAAgentError::AgentInputError("expected audio input".into())),
        }
    }

    fn describe(&self) -> String {
        format!("SpeechToTextAgent({})", self.id)
    }
}

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

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut dp: Vec<usize> = (0..=b.len()).collect();

    for (i, &ca) in a.iter().enumerate() {
        let mut prev = dp[0];
        dp[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let tmp = dp[j + 1];
            dp[j + 1] = (dp[j + 1] + 1) // deletion
                .min(dp[j] + 1) // insertion
                .min(prev + cost); // substitution
            prev = tmp;
        }
    }

    dp[b.len()]
}

// TODO: Move this to the HTTP or Socket endpoint
// TODO: When glasses are there we will use a trained model for wake word detection
fn rewrite_wake_word(transcript: &str, wake_word: &str, max_distance: usize) -> String {
    let trimmed = transcript.trim();
    let mut iter = trimmed.split_whitespace();
    if iter.next().is_none() {
        return transcript.to_string();
    }
    let raw_second = match iter.next() {
        Some(word) => word,
        None => return transcript.to_string(),
    };
    let clean_second = raw_second.trim_matches(|c: char| !c.is_alphanumeric());
    if clean_second.is_empty() {
        return transcript.to_string();
    }
    if levenshtein(wake_word, clean_second) > max_distance {
        return transcript.to_string();
    }
    if let Some(idx) = trimmed.find(raw_second) {
        let before = &trimmed[..idx];
        let after = &trimmed[idx + raw_second.len()..];
        let (leading, trailing) = if let Some(inner_idx) = raw_second.find(clean_second) {
            let leading = &raw_second[..inner_idx];
            let trailing = &raw_second[inner_idx + clean_second.len()..];
            (leading, trailing)
        } else {
            ("", "")
        };
        let mut out = String::with_capacity(
            before.len() + leading.len() + wake_word.len() + trailing.len() + after.len(),
        );
        out.push_str(before);
        out.push_str(leading);
        out.push_str(wake_word);
        out.push_str(trailing);
        out.push_str(after);
        out
    } else {
        transcript.to_string()
    }
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

    #[test]
    fn test_levenshtein() {
        let dist = levenshtein("Solia", "Soya");
        assert_eq!(dist, 2);
    }

    // TODO: set a flag or ignore for when a CI environment is used
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

    // TODO: Move this when the main function is moved
    #[test]
    fn test_rewrite_wake_word() {
        let wake = "Solia";

        let t1 = "Hey Soya";
        let r1 = rewrite_wake_word(t1, wake, 2);
        assert_eq!(r1, "Hey Solia");

        let t2 = "Hey, Soya!";
        let r2 = rewrite_wake_word(t2, wake, 2);
        assert_eq!(r2, "Hey, Solia!");

        let t3 = "Hello there";
        let r3 = rewrite_wake_word(t3, wake, 2);
        assert_eq!(r3, "Hello there");
    }
}
