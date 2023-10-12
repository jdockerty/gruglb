use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::{collections::HashMap, fs::File};
use tracing_subscriber::filter::LevelFilter;

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
    /// The key of the HashMap structure is a simple convenience label for
    /// configuration and access.
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
    pub listener: u16,
    pub backends: Option<Vec<Backend>>,
    // routing_algorithm: RoutingAlgorithm,
}

// An instance for a backend server that will have traffic routed to.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backend {
    pub host: String,
    pub port: String,
    pub healthcheck_path: String,
    // healthcheck_interval: <type>
}

// Choice of a variety of routing algorithms.
pub enum RoutingAlgorithm {
    Simple,
}

pub fn new(config_file: File) -> Result<Config> {
    let conf: Config = serde_yaml::from_reader(config_file)?;
    Ok(conf)
}

impl Config {
    /// Retrieve the configured ports in ascending order.
    pub fn ports(&self) -> Option<Vec<u16>> {
        if let Some(targets) = &self.targets {
            let mut ports = vec![];
            for (_, t) in targets {
                ports.push(t.listener);
            }
            ports.sort();
            Some(ports)
        } else {
            None
        }
    }

    /// Retrieve a list of names given to targets.
    pub fn target_names(&self) -> Option<Vec<String>> {
        if let Some(targets) = &self.targets {
            let mut names = vec![];
            for (name, _) in targets {
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
    fn config_ports_match() {
        let test_config = File::open("tests/fixtures/example-config.yaml").unwrap();

        let conf = new(test_config).unwrap();

        let expected_ports: Vec<u16> = vec![9090, 9091];
        let actual_ports = conf.ports().unwrap();

        assert_eq!(actual_ports, expected_ports);
    }

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
