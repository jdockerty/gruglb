mod config;
mod lb;
mod proxy;

use anyhow::Result;
use clap::Parser;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::File;

use std::path::PathBuf;
use std::thread;
use tracing::info;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Path to the gruglb config file.
    #[arg(short, long)]
    config: PathBuf,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let config_file = File::open(args.config)?;
    let lb = lb::new(config::new(config_file)?, true);

    lb.run();

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
