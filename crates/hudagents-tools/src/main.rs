use clap::{Parser, Subcommand};
use sysinfo::System;
use whisper_rs::{print_system_info, SystemInfo};

#[derive(Parser)]
#[command {name = "hudagents-tools"}]
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
    
    let mut sys = System::new_all();
    sys.refresh_all();
    let total_ram_gb = sys.total_memory() / 1024 / 1024 / 1024;
    println!("Total RAM: {} GB", total_ram_gb);
    let cpu_info = SystemInfo::default();
    println!("\n--- Recommendation ---");
    match resolve_backend() {
        Backend::AppleSilicon => {
            println!("Detected Apple Silicon (M-series). Metal acceleration available.");
            match total_ram_gb {
                0..=7 => unreachable!(),
                8..=16 => "small",
                17..=32 => "medium",   
                33..=64 => "large",   
                65..=128 => "large-v3", 
                129..=u64::MAX => "large-v3",
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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sysinfo => {
            let recommendation = sysinfo();
            println!("Recommended model: {}", recommendation);
        }   
        Commands::Download { model, path } => {
            println!("Downloading model {} to {}", model, path.unwrap_or_default());
        }
    }
}
