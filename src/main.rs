mod config;
mod proxy;

use anyhow::Result;
use clap::Parser;
use config::{Backend, Target};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::File;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, RwLock};

use std::path::PathBuf;
use std::thread;
use tracing::{debug, error, info};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Path to the gruglb config file.
    #[arg(short, long)]
    config: PathBuf,
}

static TCP_HEALTHY_TARGETS: Lazy<Arc<RwLock<HashMap<String, Vec<Backend>>>>> = Lazy::new(|| {
    let h = HashMap::new();
    Arc::new(RwLock::new(h))
});

fn main() -> Result<()> {
    let args = Cli::parse();
    let config_file = File::open(args.config)?;
    let conf = Box::new(config::new(config_file)?);
    let listen_addr = conf.address.clone();
    let idx: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));

    FmtSubscriber::builder()
        .with_max_level(conf.log_level())
        .init();

    if let Some(targets) = &conf.targets {
        if let Some(target_names) = conf.target_names() {
            debug!("All loaded targets {:?}", target_names);
        }

        // Provides the health check thread with its own configuration.
        let health_check_conf = conf.clone();
        let tcp_targets = Arc::clone(&TCP_HEALTHY_TARGETS);
        thread::spawn(move || proxy::tcp_health(health_check_conf, tcp_targets));

        for (listener, target) in targets.clone() {
            let addr = format!("{}:{}", listen_addr.clone(), listener);
            let listener = TcpListener::bind(&addr)?;
            info!("Listening on {} for {}", &addr, &target.name);

            // Listen to incoming traffic on separate threads
            let idx = idx.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            let idx = idx.clone();
                            let tcp_targets = Arc::clone(&TCP_HEALTHY_TARGETS);
                            let target_name = target.clone().name;
                            // Pass the TCP streams over to separate threads to avoid
                            // blocking and give each thread its copy of the configuration.
                            thread::spawn(move || {
                                proxy::tcp_connection(tcp_targets, target_name, idx, stream)
                            });
                        }
                        Err(e) => {
                            error!("Unable to connect: {}", e);
                        }
                    }
                }
            });
        }
    } else {
        info!("No listeners configured, nothing to do.");
        return Ok(());
    }

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
