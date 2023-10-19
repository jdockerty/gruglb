use anyhow::Result;
use serde::{Deserialize, Serialize};

use std::{collections::HashMap, fs::File};
use tracing_subscriber::filter::LevelFilter;

/// Protocol to use against a configured target.
#[allow(dead_code)]
pub enum Protocol {
    Tcp,
    Http,
}

// Represents the load balancer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Bind address of the application, defaults to 127.0.0.1 if not set.
    #[serde(default = "default_address")]
    pub address: String,

    /// Log level of the application, defaults to INFO.
    #[serde(default = "default_logging")]
    pub logging: String,

    /// The configured targets by the user.
    /// This provides a mapping between a convenient name and its
    /// configured targets.
    pub targets: Option<HashMap<String, Target>>,
}

/// Default for the address binding of the application when not set.
fn default_address() -> String {
    "127.0.0.1".to_string()
}

/// Default log level of the application when not set.
fn default_logging() -> String {
    "INFO".to_string()
}

// A target encapsulates a port that the load balancer listens on for forwarding
// traffic to configured backend servers.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Target {
    // Protocol to use for the target's backends.
    pub protocol: String,

    // Listener to use with TCP.
    pub listener: Option<u16>,

    /// Backends to route traffic to.
    pub backends: Option<Vec<Backend>>,
    // routing_algorithm: RoutingAlgorithm,
}

// An instance for a backend server that will have traffic routed to.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backend {
    pub host: String,
    pub port: u16,
    pub healthcheck_path: Option<String>,
}

impl PartialEq for Backend {
    fn eq(&self, other: &Backend) -> bool {
        self.port == other.port
            && self.host == other.host
            && self.healthcheck_path == other.healthcheck_path
    }
}

// Choice of a variety of routing algorithms.
#[allow(dead_code)]
pub enum RoutingAlgorithm {
    Simple,
}

pub fn new(config_file: File) -> Result<Config> {
    let conf: Config = serde_yaml::from_reader(config_file)?;
    Ok(conf)
}

impl Config {
    /// Retrieve a list of names given to targets.
    pub fn target_names(&self) -> Option<Vec<String>> {
        if let Some(targets) = &self.targets {
            let mut names = vec![];
            for name in targets.keys() {
                names.push(name.to_string());
            }
            Some(names)
        } else {
            None
        }
    }

    pub fn log_level(&self) -> LevelFilter {
        match self.logging.to_uppercase().as_str() {
            "TRACE" => LevelFilter::TRACE,
            "DEBUG" => LevelFilter::DEBUG,
            _ => LevelFilter::INFO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_targets_match() {
        let test_config = File::open("tests/fixtures/example-config.yaml").unwrap();

        let conf = new(test_config).unwrap();

        let names = conf.target_names().unwrap();

        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|elem| elem == "webServersA"));
        assert!(names.iter().any(|elem| elem == "webServersB"));
    }
}
