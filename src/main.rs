mod config;

use anyhow::Result;
use clap::Parser;
use rand::prelude::*;
use serde_json::json;
use std::fs::File;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Path to the gruglb config file.
    #[arg(short, long)]
    config: PathBuf,
}

fn work(name: &str, stream: TcpStream) {
    println!("{name} starting work");
    let res = json!({"status": "OK"});
    thread::sleep(std::time::Duration::from_secs(5));
    let _ = serde_json::to_writer(stream, &res);
    println!("{name} done!");
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let config_file = File::open(args.config)?;

    let conf = config::new(config_file)?;

    if let Some(targets) = &conf.targets {
        for (name, target) in targets {
            let addr = format!("{}:{}", conf.address, target.listener);
            let listener = TcpListener::bind(&addr).unwrap();
            println!("Listening on {} for {name}", addr);

            // Listen to incoming traffic on separate threads
            thread::spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            // Pass the TCP streams over to separate threads to avoid
                            // blocking.
                            thread::spawn(move || {
                                let mut rng = thread_rng();
                                let n: u32 = rng.gen();
                                let name = &format!("thread-{}", n);
                                work(name, stream);
                            });
                        }
                        Err(e) => {
                            eprintln!("Unable to connect: {}", e);
                        }
                    }
                }
            });
        }
    } else {
        eprintln!("No listeners configured, nothing to do.");
        return Ok(());
    }

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
