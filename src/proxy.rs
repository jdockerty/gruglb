use crate::config::{Backend, Config, Protocol, Target};
use crate::lb::SendTargets;
use anyhow::Result;
use std::io::BufReader;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::Duration,
    vec,
};
use tracing::{debug, error, info};

// HTTP health checks for L7 targets.
// TODO: refactor common functionality between TCP/HTTP targets.
//pub fn http_health(conf: Box<Config>) {
//    let duration = Duration::from_secs(2);
//    let health_client = reqwest::blocking::Client::builder()
//        .timeout(Duration::from_secs(1))
//        .build()
//        .unwrap();
//
//    if let Some(targets) = &conf.targets {
//        let current_healthy_targets: Arc<Mutex<HashMap<String, Vec<Backend>>>> =
//            Arc::new(Mutex::new(HashMap::new()));
//
//        info!("Starting HTTP health checks");
//        loop {
//            debug!("Healthy HTTP {:?}", current_healthy_targets);
//            for target in targets.values() {
//                if let Some(backends) = &target.backends {
//                    for backend in backends {
//                        let health_uri = &format!(
//                            "http://{}:{}{}",
//                            backend.host, backend.port, backend.healthcheck_path
//                        );
//                        let mut healthy_targets = match current_healthy_targets.lock() {
//                            Ok(healthy_targets) => healthy_targets,
//                            Err(e) => {
//                                error!("Unable to acquire lock: {e}");
//                                e.into_inner()
//                            }
//                        };
//                        // Simple blocking call to avoid async functions leaking everywhere.
//                        match health_client.get(health_uri).send() {
//                            Ok(response) => {
//                                if response.status().is_success()
//                                    || response.status().is_redirection()
//                                {
//                                    info!("{health_uri} is healthy backend for {}", target.name);
//                                }
//
//                                if response.status().is_client_error()
//                                    || response.status().is_server_error()
//                                {
//                                    debug!(
//                                        "{} is unhealthy for {health_uri} ({}), attempting to remove",
//                                        target.name,
//                                        response.status()
//                                    );
//                                }
//                            }
//                            Err(err) => {
//                                error!(
//                                    "Unable to health check {health_uri}, attempting to remove {}: {err}",
//                                    target.name
//                                );
//                            }
//                        }
//                    }
//                } else {
//                    info!("No backends to health check for {}", target.name);
//                }
//            }
//            thread::sleep(duration);
//        }
//    } else {
//        info!("No targets configured, unable to health check.");
//    }
//}

/// Run health checks against the configured TCP targets.
pub fn tcp_health(conf: Arc<Config>, sender: SendTargets) {
    let duration = Duration::from_secs(5);

    if let Some(targets) = &conf.targets {
        info!("Starting TCP health checks");
        loop {
            let mut healthy_backends: Vec<Backend> = vec![];
            let mut healthy_targets = HashMap::new();
            for (name, target) in targets {
                healthy_targets.insert(name.to_string(), healthy_backends.clone());
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!("{}:{}", backend.host, backend.port);

                        if let Ok(stream) = TcpStream::connect(request_addr) {
                            stream.shutdown(Shutdown::Both).unwrap();
                            info!("{request_addr} is healthy backend for {}", name);
                            healthy_backends.push(backend.clone());
                        } else {
                            // This is "removed from the pool" because it is not included in
                            // the vector for the next channel transmission, so traffic does not get routed
                            // to it.
                            debug!(
                                "{request_addr} is unhealthy for {}, removing from pool",
                                name
                            );
                        }
                    }
                    healthy_targets.insert(name.to_string(), healthy_backends.clone());
                } else {
                    info!("No backends to health check for {}", name);
                }
            }
            debug!("Sending targets to channel");
            sender.send(healthy_targets).unwrap();
            thread::sleep(duration);
        }
    } else {
        info!("No targets configured, unable to health check.");
    }
}

// Proxy a TCP connection to a range of configured backend servers.
pub fn tcp_connection<S>(
    targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    target_name: String,
    routing_idx: Arc<Mutex<usize>>,
    mut stream: S,
) -> Result<()>
where
    S: Read + Write,
{
    if let Some(backends) = targets.read().unwrap().get(&target_name) {
        let backends = backends.to_vec();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        if backend_count == 0 {
            info!("[TCP] No routable backends for {target_name}, nothing to do");
            return Ok(());
        }

        let mut idx = match routing_idx.lock() {
            Ok(idx) => idx,
            Err(poisoned) => poisoned.into_inner(),
        };

        debug!("[TCP] {backend_count} backends configured for {target_name}, current index {idx}");

        // Reset index when out of bounds to route back to the first server.
        if *idx >= backend_count {
            *idx = 0;
        }

        let backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);

        // Increment a shared index after we've constructed our current connection
        // address.
        *idx += 1;

        info!("[TCP] Attempting to connect to {}", &backend_addr);
        match TcpStream::connect(backend_addr) {
            Ok(mut response) => {
                let mut buffer = Vec::new();
                response.read_to_end(&mut buffer)?;
                stream.write_all(&buffer)?;
                debug!("TCP stream closed");
            }
            Err(e) => {
                error!("{e}")
            }
        };
    } else {
        info!("[TCP] No backend configured");
    };

    Ok(())
}

pub fn http_connection<S>(
    targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    target_name: String,
    routing_idx: Arc<Mutex<usize>>,
    method: String,
    request_path: String,
    mut stream: S,
) -> Result<()>
where
    S: Read + Write,
{
    if let Some(backends) = targets.read().unwrap().get(&target_name) {
        let backends = backends.to_vec();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        if backend_count == 0 {
            info!("[HTTP] No routable backends for {target_name}, nothing to do");
            return Ok(());
        }

        let mut idx = match routing_idx.lock() {
            Ok(idx) => idx,
            Err(poisoned) => poisoned.into_inner(),
        };

        debug!("[HTTP] {backend_count} backends configured for {target_name}, current index {idx}");

        // Reset index when out of bounds to route back to the first server.
        if *idx >= backend_count {
            *idx = 0;
        }

        let http_backend = format!(
            "http://{}:{}{}",
            backends[*idx].host, backends[*idx].port, request_path
        );

        // Increment a shared index after we've constructed our current connection
        // address.
        *idx += 1;

        info!("[HTTP] Attempting to connect to {}", &http_backend);
        let client = reqwest::blocking::Client::new();

        match method.as_str() {
            "GET" => {
                let backend_response = client.get(http_backend).send()?;

                let version = backend_response.version();
                debug!("RESP: {:?}", backend_response.text()?);
                // TODO: Send response back to caller via stream.
            }
            "POST" => {
                let response = client.post(http_backend).send()?;
                stream.write_all(&response.bytes()?)?;
            }
            _ => {
                error!("Unsupported: {method}")
            }
        }
    } else {
        info!("[HTTP] No backend configured");
    };

    Ok(())
}

/// Bind to the configured listener ports for incoming TCP connections.
fn get_tcp_listeners(
    bind_address: String,
    targets: HashMap<String, Target>,
) -> Result<Vec<(String, TcpListener)>> {
    let mut tcp_bindings = vec![];

    for (name, target) in targets {
        if target.protocol_type() == Protocol::Tcp {
            let addr = format!("{}:{}", bind_address.clone(), target.listener.unwrap());
            let listener = TcpListener::bind(&addr)?;
            info!("Binding to {} for {}", &addr, &name);
            tcp_bindings.push((name, listener));
        }
    }

    Ok(tcp_bindings)
}

fn get_http_listeners(
    bind_address: String,
    targets: HashMap<String, Target>,
) -> Result<Vec<(String, TcpListener)>> {
    let mut http_bindings = vec![];

    for (name, target) in targets {
        if target.protocol_type() == Protocol::Http {
            let addr = format!("{}:{}", bind_address.clone(), target.listener.unwrap());
            let listener = TcpListener::bind(&addr)?;
            info!("Binding to {} for {}", &addr, &name);
            http_bindings.push((name, listener));
        }
    }

    Ok(http_bindings)
}

pub fn accept_http(
    bind_address: String,
    current_healthy_targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    targets: HashMap<String, Target>,
) -> Result<()> {
    let idx: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let bound_listeners = get_http_listeners(bind_address, targets)?;

    for (name, listener) in bound_listeners {
        for stream in listener.incoming() {
            let name = name.clone();
            let idx = Arc::clone(&idx);
            let current_healthy_targets = Arc::clone(&current_healthy_targets);
            thread::spawn(move || match stream {
                Ok(mut stream) => {
                    info!("Incoming HTTP request");
                    use std::io::prelude::*;
                    let buf = BufReader::new(&mut stream);

                    let http_request: Vec<_> = buf
                        .lines()
                        .map(|result| result.unwrap())
                        .take_while(|line| !line.is_empty())
                        .collect();

                    let info = http_request[0].clone();
                    let http_info = info
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>();

                    let method = http_info[0].clone();
                    let path = http_info[1].clone();

                    debug!("{method} request at {path}");
                    thread::spawn(move || {
                        http_connection(
                            current_healthy_targets,
                            name,
                            idx,
                            method.to_string(),
                            path.to_string(),
                            stream,
                        )
                    });
                }
                Err(e) => {
                    error!("Unable to connect: {}", e);
                }
            });
        }
    }

    Ok(())
}

/// Accept TCP connections by binding to multiple `TcpListener` socket address and
/// handling incoming connections, passing them to the configured TCP backends.
pub fn accept_tcp(
    bind_address: String,
    current_healthy_targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    targets: HashMap<String, Target>,
) -> Result<()> {
    let idx: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let bound_listeners = get_tcp_listeners(bind_address, targets)?;

    for (name, listener) in bound_listeners {
        // Listen to incoming traffic on separate threads
        let idx = Arc::clone(&idx);
        let current_healthy_targets = Arc::clone(&current_healthy_targets);
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let request_port = stream.local_addr().unwrap().port();
                        info!("Incoming request on {}", &request_port);

                        let idx = Arc::clone(&idx);
                        let tcp_targets = Arc::clone(&current_healthy_targets);
                        // Pass the TCP streams over to separate threads to avoid
                        // blocking and give each thread its copy of the configuration.
                        let target_name = name.clone();
                        thread::spawn(move || {
                            tcp_connection(tcp_targets, target_name, idx, stream)
                        });
                    }
                    Err(e) => {
                        error!("Unable to connect: {}", e);
                    }
                }
            }
        });
    }

    Ok(())
}

//#[cfg(test)]
//mod tests {
//    #[test]
//    fn tcp_health_registers_correctly() {
//        todo!()
//    }
//}
