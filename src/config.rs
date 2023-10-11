use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::fs::File;

// Represents the load balancer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // The configured targets by the user.
    pub targets: Option<Vec<Target>>,
}

// A target encapsulates a port that the load balancer listens on for forwarding
// traffic to configured backend servers.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Target {
    listener: u16,
    backends: Option<Vec<Backend>>,
    // routing_algorithm: RoutingAlgorithm,
}

// An instance for a backend server that will have traffic routed to.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Backend {
    host: String,
    port: String,
    healthcheck_path: String,
    // healthCheckInterval: <type>
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
            for t in targets {
                ports.push(t.listener);
            }
            ports.sort();
            Some(ports)
        } else {
            None
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

        assert_eq!(expected_ports, actual_ports);
    }
}
