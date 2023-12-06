pub mod config;
pub mod fakebackend;
pub mod http;
pub mod lb;
pub mod proxy;
pub mod tcp;

use anyhow::Result;
use clap::Parser;
use lb::LB;
use std::fs::File;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Path to the gruglb config file.
    #[arg(short, long)]
    config: PathBuf,
}

// Initialise core application logic and returning the load balancer ready to run.
pub fn init() -> Result<LB> {
    let args = Cli::parse();
    let config_file = File::open(args.config)?;

    Ok(LB::new(config::new(config_file)?))
}
