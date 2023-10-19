mod config;
mod lb;
mod proxy;

use anyhow::Result;
use clap::Parser;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::thread;

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
    let channel_size = lb.conf.target_names().map_or(2, |targets| targets.len());
    let (send, recv): (lb::SendTargets, lb::RecvTargets) = sync_channel(channel_size);

    lb.run(send, recv)?;

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
