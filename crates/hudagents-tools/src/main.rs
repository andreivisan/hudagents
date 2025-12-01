use clap::{Parser, Subcommand};
use sysinfo::System;
use whisper_rs::print_system_info;

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

// TODO: Use a DEBUG_LEVEL flag to control the verbosity of the output
fn sysinfo() {
    println!("--- Whisper System Info ---");
    println!("{}", print_system_info());
    
    let mut sys = System::new_all();
    sys.refresh_all();
    let total_ram_gb = sys.total_memory() / 1024 / 1024 / 1024;
    println!("Total RAM: {} GB", total_ram_gb);
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sysinfo => {
            sysinfo();
        }   
        Commands::Download { model, path } => {
            println!("Downloading model {} to {}", model, path.unwrap_or_default());
        }
    }
}
