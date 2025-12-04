use clap::{Parser, Subcommand};
use hudagents_local::whisper::HAWhisperError;
use sysinfo::{System};
use std::{
    env,
    fs::File,
    io::copy,
    path::{Path, PathBuf}, 
    result::Result,
};
use whisper_rs::{print_system_info, SystemInfo};

const AVAILABLE_MODELS: &[&str] = &[
    "tiny", "tiny.en", "tiny-q5_1", "tiny.en-q5_1", "tiny-q8_0",
    "base", "base.en", "base-q5_1", "base.en-q5_1", "base-q8_0",
    "small", "small.en", "small.en-tdrz", "small-q5_1", "small.en-q5_1", "small-q8_0",
    "medium", "medium.en", "medium-q5_0", "medium.en-q5_0", "medium-q8_0",
    "large-v1", "large-v2", "large-v2-q5_0", "large-v2-q8_0",
    "large-v3", "large-v3-q5_0", "large-v3-turbo", "large-v3-turbo-q5_0", "large-v3-turbo-q8_0"
];

#[derive(Parser)]
#[command (name = "hudagents-tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Sysinfo,
    Download {
        #[arg(long)]
        model: String,
        #[arg(long)]
        path: Option<String>,
    }
}

enum Backend {
    AppleSilicon,
    IntelMac,
    Cuda,
    Vulkan,
    CPUOnly,
}

fn resolve_backend() -> Backend {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        Backend::AppleSilicon
    } else if cfg!(target_os = "macos") {
        Backend::IntelMac
    } else if cfg!(feature = "cuda") {
        Backend::Cuda
    } else if cfg!(feature = "vulkan") {
        Backend::Vulkan
    } else {
        Backend::CPUOnly
    }
}

// TODO: Use a DEBUG_LEVEL flag to control the verbosity of the output
fn sysinfo() -> &'static str{
    println!("--- Whisper System Info ---");
    println!("{}", print_system_info());
    
    let mut sys = System::new();
    sys.refresh_memory();
    let total_ram_gb = sys.total_memory() / 1024 / 1024 / 1024;
    println!("Total RAM: {} GB", total_ram_gb);
    let cpu_info = SystemInfo::default();
    println!("\n--- Recommendation ---");
    match resolve_backend() {
        Backend::AppleSilicon => {
            println!("Detected Apple Silicon (M-series). Metal acceleration available.");
            match total_ram_gb {
                0..=7 => "base",
                8..=16 => "small",
                17..=32 => "medium",   
                33..=64 => "large",   
                _ => "large-v3",
            }
        }
        Backend::IntelMac => {
            println!("Detected Intel Mac.");
            match (total_ram_gb, cpu_info.avx2) {
                (16.., true) => "medium",
                _ => "base",
            }
        }
        Backend::Cuda => {
            println!("Binary compiled with CUDA support.");
            "large-v3"
        }
        Backend::Vulkan => {
            println!("Binary compiled with Vulkan support.");
            match total_ram_gb {
                8.. => "medium",
                _ => "small",
            }
        }
        Backend::CPUOnly => {
            println!("Running on CPU.");
            match (total_ram_gb, cpu_info.avx2) {
                (8.., true) => "small",
                _ => "base",
            }
        }
    }
}

fn determine_download_url(model: &str) -> (&'static str, &'static str) {
    match model.contains("tdrz") {
        true => ("https://huggingface.co/akashmjn/tinydiarize-whisper.cpp", "resolve/main/ggml"),
        false => ("https://huggingface.co/ggerganov/whisper.cpp", "resolve/main/ggml"),
    }
}

fn download_model(model: &str, custom_path: Option<&Path>) -> Result<(), HAWhisperError> {
    if !AVAILABLE_MODELS.contains(&model) {
        return Err(HAWhisperError::InvalidModelName(model.to_string()));
    }
    //TODO: Maybe in the future consider directories crate for multi platform support
    let target_dir = match custom_path {
        Some(path) => PathBuf::from(path),
        None => {
            if let Some(env_path) = env::var_os("HA_WHISPER_PATH") { PathBuf::from(env_path) }
            else { PathBuf::from(".models") }
        },
    };
    //TODO: If target_dir does not exist, create it
    let filename = format!("{model}.bin");
    let file_path = target_dir.join(&filename);
    if file_path.exists() {
        println!("Model {} already exists at {:?}. Skipping download.", model, file_path);
        return Ok(());
    }
    let (base_url, prefix) = determine_download_url(model);
    let url = format!("{base_url}/{prefix}-{model}.bin");
    println!("Downloading model {} from '{}' ...", model, url);
    println!("Saving to {:?}", file_path);
    let mut response = reqwest::blocking::get(url).map_err(HAWhisperError::HttpRequestFailed)?;
    if !response.status().is_success() { return Err(HAWhisperError::HttpStatus(response.status())); }
    let mut dest_file = match File::create(&file_path) {
        Ok(file) => file,
        Err(e) => return Err(HAWhisperError::IOError(e)),
    };
    match copy(&mut response, &mut dest_file) {
        Ok(_) => println!("Model downloaded successfully."),
        Err(e) => return Err(HAWhisperError::IOError(e)),
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sysinfo => {
            let recommendation = sysinfo();
            println!("Recommended model: {}", recommendation);
        }   
        Commands::Download { model, path } => {
            println!("Downloading model {} to {}", model, path.unwrap_or_default());
            match download_model(&model, None) {
                Ok(_) => println!("Model downloaded successfully."),
                Err(e) => println!("Error downloading model: {}", e),
            }
        }
    }
}
