use clap::{Parser, Subcommand};

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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sysinfo => {
            println!("Running system info...");
        }   
        Commands::Download { model, path } => {
            println!("Downloading model {} to {}", model, path.unwrap_or_default());
        }
    }
}
