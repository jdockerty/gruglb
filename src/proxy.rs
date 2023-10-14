use crate::config::Config;
use reqwest;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tracing::{debug, error, info};

/// Run health checks against configured targets.
pub fn health_check(conf: Box<Config>) {
    let duration = Duration::from_secs(1);

    if let Some(targets) = &conf.targets {
        loop {
            thread::sleep(duration);
            for target in targets.values() {
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!(
                            "{}:{}{}",
                            backend.host, backend.port, backend.healthcheck_path
                        );
                        // Simple blocking call to avoid async functions leaking everywhere.
                        match reqwest::blocking::get(request_addr) {
                            Ok(response) => {
                                if response.status().is_success()
                                    || response.status().is_redirection()
                                {
                                    continue;
                                }

                                let status_code = response.status();
                                info!("{} has unhealthy backend {} ({status_code}), removing from pool", target.name, request_addr);
                            }
                            Err(err) => error!("Unable to perform health check: {err}"),
                        }
                    }
                } else {
                    info!("No backends to health check for {}", target.name);
                }
            }
        }
    } else {
        info!("No targets configured, unable to health check.");
    }
}

// Proxy a TCP connection to a range of configured backend servers.
pub fn tcp_connection(conf: Box<Config>, routing_idx: Arc<Mutex<usize>>, mut stream: TcpStream) {
    let request_port = stream.local_addr().unwrap().port();
    info!("Incoming request on {}", &request_port);

    // We can unwrap here because we have already checked for the existence of targets.
    let targets = conf.targets.unwrap();

    if let Some(target) = targets.get(&request_port) {
        let backends = target.backends.clone().unwrap();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        let mut idx = match routing_idx.lock() {
            Ok(idx) => idx,
            Err(poisoned) => poisoned.into_inner(),
        };

        debug!(
            "{backend_count} backends configured for {}, current index {idx}",
            &request_port
        );

        // Reset index when out of bounds to route back to the first server.
        if *idx >= backend_count {
            *idx = 0;
        }

        let backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);
        *idx += 1;

        info!("Attempting to connect to {}", &backend_addr);
        match TcpStream::connect(backend_addr) {
            Ok(mut response) => {
                let mut buffer = Vec::new();
                response.read_to_end(&mut buffer).unwrap();
                stream.write_all(&buffer).unwrap();
                stream.shutdown(Shutdown::Both).unwrap();
                debug!("TCP stream closed");
            }
            Err(e) => error!("{e}"),
        };
    } else {
        info!("No backend configured for {}", &request_port);
    };
}
