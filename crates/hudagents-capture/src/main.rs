use clap::Parser;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const DEFAULT_DURATION_SECS: u64 = 5;
const DEFAULT_OUTPUT_DIR: &str = "hudagents-capture/assets";
const VIDEO_INPUT: &str = "AVFoundation video devices:";
const AUDIO_INPUT: &str = "AVFoundation audio devices:";

enum DeviceSection {
    None,
    Video,
    Audio,
}

fn list_input_devices() -> std::io::Result<(Vec<String>, Vec<String>)> {
    let output = Command::new("ffmpeg")
        .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut section = DeviceSection::None;
    let mut video_devices = Vec::new();
    let mut audio_devices = Vec::new();

    for line in stderr.lines() {
        if line.contains(VIDEO_INPUT) {
            section = DeviceSection::Video;
            continue;
        }
        if line.contains(AUDIO_INPUT) {
            section = DeviceSection::Audio;
            continue;
        }
        if let Some(idx) = line.find("]") {
            match section {
                DeviceSection::Video => video_devices.push(line[idx + 1..].trim().to_owned()),
                DeviceSection::Audio => audio_devices.push(line[idx + 1..].trim().to_owned()),
                DeviceSection::None => {}
            }
        }
    }
    Ok((video_devices, audio_devices))
}

fn capture_image_and_audio(duration: u64, session: &str, output_dir: &Path) {
    let photo_output_path = output_dir.join(format!("{session}.jpg"));
    let audio_output_path = output_dir.join(format!("{session}.m4a"));
    if let Err(err) = std::fs::create_dir_all(output_dir) {
        eprintln!("could not create output dir {output_dir:?}: {err}");
        return;
    }
    let input_device = "0:2";
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
            input_device,
            // --- OUTPUT 1: The Photo ---
            "-map",
            "0:v",
            "-frames:v",
            "1",
            "-update",
            "1",
            "-y",
            photo_output_path.to_str().unwrap(),
            // --- OUTPUT 2: The Audio ---
            "-map",
            "0:a",
            "-t",
            &duration.to_string(),
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-y",
            audio_output_path.to_str().unwrap(),
        ])
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

    match list_input_devices() {
        Ok((video_devices, audio_devices)) => {
            println!("Available video input devices: {:?}", video_devices);
            println!("Available audio input devices: {:?}", audio_devices);
        }
        Err(e) => eprintln!("Error listing video input devices: {}", e),
    };

    capture_image_and_audio(args.duration, &args.session, &args.output_dir);
}
