mod config;

use anyhow::Result;
use clap::Parser;
use serde_json::json;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::AsFd;
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

fn proxy(conf: &config::Config, mut stream: TcpStream) {
    stream.set_nonblocking(true).unwrap();
    let request_port = stream.local_addr().unwrap().port();
    info!("Incoming request on {}", &request_port);

    if let Some(targets) = &conf.targets {
        if let Some(target) = targets.get(&request_port) {
            let backend = &target.backends.clone().unwrap()[0];
            debug!("Retrieved backend {:?}", &backend);

            let backend_addr = format!("{}:{}", backend.host, backend.port);
            debug!("Attempting to connect to {}", &backend_addr);

            match TcpStream::connect(backend_addr) {
                Ok(mut response) => {
                    let mut buffer = Vec::new();
                    response.read_to_end(&mut buffer).unwrap();
                    stream.write_all(&buffer).unwrap();
                    stream.shutdown(Shutdown::Both).unwrap();
                }
                Err(e) => eprintln!("{e}"),
            };
        } else {
            info!("No backend configured for {}", &request_port);
        };
    };
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
    let listen_addr = conf.address.clone();

    let _ = FmtSubscriber::builder()
        .with_max_level(conf.log_level())
        .init();

    if let Some(targets) = &conf.targets {
        if let Some(target_names) = conf.target_names() {
            debug!("All loaded targets {:?}", target_names);
        }

        for (listener, target) in targets.clone() {
            let addr = format!("{}:{}", listen_addr.clone(), listener);
            let listener = TcpListener::bind(&addr)?;
            info!("Listening on {} for {}", &addr, &target.name);

            let conf = conf.clone();
            // Listen to incoming traffic on separate threads
            thread::spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            // Pass the TCP streams over to separate threads to avoid
                            // blocking and give each thread its copy of the configuration.
                            let conf = conf.clone();
                            thread::spawn(move || {
                                proxy(&conf, stream);
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
