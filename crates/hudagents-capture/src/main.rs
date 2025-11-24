use clap::Parser;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const DEFAULT_DURATION_SECS: u64 = 10;
const DEFAULT_OUTPUT_DIR: &str = "hudagents-capture/assets";

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
        if line.contains("AVFoundation audio devices:") {
            break;
        }
        if in_video_section && let Some(idx) = line.find("]") {
            devices.push(line[idx + 1..].trim().to_owned());
        }
    }
    Ok(devices)
}

fn capture_image(device_index: usize, session: &str, output_dir: &Path) {
    let input = format!("{}:none", device_index);
    let output_path = output_dir.join(format!("{session}.jpg"));
    if let Err(err) = std::fs::create_dir_all(output_dir) {
        eprintln!("could not create output dir {output_dir:?}: {err}");
        return;
    }
    Command::new("ffmpeg")
        .args(&[
            "-f",
            "avfoundation",
            "-video_size",
            "1920x1080",
            "-framerate",
            "30",
            "-pixel_format",
            "uyvy422",
            "-i",
            &input,
            "-update",
            "1",
            "-frames:v",
            "1",
        ])
        .arg(output_path.as_os_str())
        .status()
        .expect("failed to capture photo");
}

#[derive(Parser)]
struct Args {
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

    capture_image(0, &args.session, &args.output_dir);
}
