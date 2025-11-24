use clap::Parser;
use std::{path::PathBuf, process::Command};

const DEFAULT_DURATION_SECS: u64 = 10;
const DEFAULT_OUTPUT_DIR: &str = "../assets";

fn list_video_input_devices() -> std::io::Result<Vec<String>> {
    let output = Command::new("ffmpeg")
        .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut devices = Vec::new();
    let mut in_video_section = false;
    for line in stderr.lines() {
        if line.contains("AVFoundation video devices:") {
            in_video_section = true;
            continue;            
        } 
        if line.contains("AVFoundation audio devices:") { break; }
        if in_video_section && let Some(idx) = line.find("]") { devices.push(line[idx+1..].trim().to_owned()); }
    }
    Ok(devices)
}

// pub fn capture_image(session: &str, output_dir: &PathBuf) {

// }

#[derive(Parser)]
struct Args{
    #[arg(short, long)]
    session: String,
    #[arg(short, long, default_value_t = DEFAULT_DURATION_SECS)]
    duration: u64,
    #[arg(short, long, default_value = DEFAULT_OUTPUT_DIR)]
    output_dir: PathBuf,
}

fn main() {
    let args = Args::parse();
    println!("Session: {}", args.session);
    println!("Duration: {} seconds", args.duration);
    println!("Output directory: {}", args.output_dir.display());

    let _ = match list_video_input_devices() {
        Ok(devices) => println!("Available video input devices: {:?}", devices),
        Err(e) => eprintln!("Error listing video input devices: {}", e),
    };
}
