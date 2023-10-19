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

type InitialTcpTargets = Lazy<Arc<RwLock<HashMap<String, Vec<config::Backend>>>>>;

static TCP_CURRENT_HEALTHY_TARGETS: InitialTcpTargets = Lazy::new(|| {
    let h = HashMap::new();
    Arc::new(RwLock::new(h))
});

fn main() -> Result<()> {
    let args = Cli::parse();
    let config_file = File::open(args.config)?;
    let lb = lb::new(config::new(config_file)?);
    let (tx, rx): (SendTargets, RecvTargets) = channel();

    if let Some(targets) = &conf.targets {
        proxy::run(Arc::clone(&conf), targets.clone(), tx, rx)?;
    } else {
        info!("No listeners configured, nothing to do.");
        return Ok(());
    }

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
