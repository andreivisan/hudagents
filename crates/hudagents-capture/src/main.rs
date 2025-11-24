use clap::Parser;
use std::path::PathBuf;

const DEFAULT_DURATION_SECS: u64 = 10;
const DEFAULT_OUTPUT_DIR: &str = "../assets";

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
}
