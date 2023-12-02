use anyhow::Result;
use serde::{Deserialize, Serialize};

use std::{collections::HashMap, fmt::Display, fs::File, time::Duration};
use tracing_subscriber::filter::LevelFilter;

/// Protocol to use against a configured target.
#[derive(Debug, PartialEq)]
pub enum Protocol {
    Tcp,
    Http,
    Unsupported,
}

// Represents the load balancer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Bind address of the application, defaults to 127.0.0.1.
    pub address: Option<String>,

    /// Log level of the application, defaults to INFO.
    pub logging: Option<String>,

    /// Interval, in seconds, to conduct health checks against all configured targets.
    /// Defaults to every 10 seconds when unset.
    pub health_check_interval: Option<u8>,

    /// The configured targets by the user.
    /// This provides a mapping between a convenient name and its
    /// configured targets.
    /// When no targets are provided, nothing happens.
    pub targets: Option<HashMap<String, Target>>,
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

impl Target {
    /// Retrieve the type of protocol configured for the target.
    pub fn protocol_type(&self) -> Protocol {
        match self.protocol.as_str() {
            "tcp" => Protocol::Tcp,
            "http" => Protocol::Http,
            _ => Protocol::Unsupported,
        }
    }
}

// An instance for a backend server that will have traffic routed to.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backend {
    pub host: String,
    pub port: u16,
    pub health_path: Option<String>,
}

impl PartialEq for Backend {
    fn eq(&self, other: &Backend) -> bool {
        self.port == other.port && self.host == other.host && self.health_path == other.health_path
    }
}

impl Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(health_path) = &self.health_path {
            write!(f, "{}:{}{}", self.host, self.port, health_path)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
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

    /// Utility function for parsing the log level from the configuration.
    pub fn log_level(&self) -> LevelFilter {
        match self
            .logging
            .to_owned()
            .unwrap_or_else(|| "INFO".to_string())
            .to_uppercase()
            .as_str()
        {
            "TRACE" => LevelFilter::TRACE,
            "DEBUG" => LevelFilter::DEBUG,
            "ERROR" => LevelFilter::ERROR,
            "INFO" => LevelFilter::INFO,
            _ => LevelFilter::INFO,
        }
    }

    /// Retrieve the health_check_interval as a Duration, ready to use within
    /// the application.
    pub fn health_check_interval(&self) -> Duration {
        Duration::from_secs(self.health_check_interval.unwrap_or(5).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_config() -> Config {
        let test_config = File::open("tests/fixtures/example-config.yaml").unwrap();
        new(test_config).unwrap()
    }

    #[test]
    fn named_targets_match() {
        let conf = get_config();
        let names = conf.target_names().unwrap();

        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|elem| elem == "webServersA"));
        assert!(names.iter().any(|elem| elem == "tcpServersA"));
    }

    #[test]
    fn protocol_matches() {
        let conf = get_config();

        let targets = conf.targets.unwrap();

        let http_target = &targets["webServersA"];
        let tcp_target = &targets["tcpServersA"];

        let unsupported = Target {
            protocol: "invalid_protocol".to_string(),
            listener: None,
            backends: None,
        };

        assert_eq!(http_target.protocol_type(), Protocol::Http);
        assert_eq!(tcp_target.protocol_type(), Protocol::Tcp);
        assert_eq!(unsupported.protocol_type(), Protocol::Unsupported);
    }

    #[test]
    fn health_check_interval() {
        let conf = get_config();
        let config_duration = conf.health_check_interval();
        assert_eq!(conf.health_check_interval.unwrap(), 2_u8);
        assert_eq!(config_duration, Duration::from_secs(2));
    }
}
