pub mod config;
pub mod fakebackend;
pub mod lb;
pub mod proxy;

use anyhow::Result;
use clap::Parser;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Path to the gruglb config file.
    #[arg(short, long)]
    config: PathBuf,
}

#[derive(Error, Debug)]
pub enum HTTPError {
    #[error("{0} is an unsupported HTTP method")]
    UnsupportedMethod(String),

    #[error("error proxying request to {upstream_address}: {source}")]
    ProxyRequest {
        upstream_address: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("invalid http body")]
    InvalidBody(#[source] reqwest::Error),

    #[error("unable to write response")]
    Write(#[from] io::Error),
}

#[derive(Error, Debug)]
pub enum TCPError {
    #[error("error proxying request to {upstream_address}")]
    ProxyRequest {
        upstream_address: String,
        #[source]
        source: io::Error,
    },
}

// Initialise core application logic and returning the load balancer ready to run.
pub fn init() -> Result<lb::LB> {
    let args = Cli::parse();
    let config_file = File::open(args.config)?;
    let lb = lb::new(config::new(config_file)?);

    Ok(lb)
}
