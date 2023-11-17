use crate::config::{Backend, Config, Protocol, Target};
use crate::lb::SendTargets;
use crate::{HTTPError, TCPError};
use anyhow::Result;
use reqwest::blocking::Response;
use std::io::prelude::*;
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

pub fn http_health(conf: Arc<Config>, sender: SendTargets) {
    let health_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();

    if let Some(targets) = &conf.targets {
        info!("Starting HTTP health checks");
        loop {
            let mut healthy_backends: Vec<Backend> = vec![];
            let mut healthy_targets = HashMap::new();
            for (name, target) in targets {
                if target.protocol_type() != Protocol::Http {
                    continue;
                }

                healthy_targets.insert(name.to_string(), healthy_backends.clone());
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!(
                            "http://{}:{}{}",
                            backend.host,
                            backend.port,
                            backend.health_path.clone().unwrap()
                        );

                        if let Ok(response) = health_client.get(request_addr).send() {
                            if response.status().is_success() || response.status().is_redirection()
                            {
                                info!("{request_addr} is healthy backend for {}", name);
                                healthy_backends.push(backend.clone());
                            }
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
            debug!("[HTTP] Sending targets to channel");
            sender.send(healthy_targets).unwrap();
            thread::sleep(conf.health_check_interval());
        }
    } else {
        info!("No targets configured, unable to health check.");
    }
}

/// Run health checks against the configured TCP targets.
pub fn tcp_health(conf: Arc<Config>, sender: SendTargets) {
    if let Some(targets) = &conf.targets {
        info!("Starting TCP health checks");
        loop {
            let mut healthy_backends: Vec<Backend> = vec![];
            let mut healthy_targets = HashMap::new();
            for (name, target) in targets {
                if target.protocol_type() != Protocol::Tcp {
                    continue;
                }
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
            thread::sleep(conf.health_check_interval());
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
        match TcpStream::connect(&backend_addr) {
            Ok(mut response) => {
                let mut buffer = Vec::new();
                response.read_to_end(&mut buffer)?;
                stream.write_all(&buffer)?;
                debug!("TCP stream closed");
            }
            Err(e) => {
                Err(TCPError::ProxyRequest {
                    upstream_address: backend_addr.clone(),
                    source: e,
                })?;
            }
        };
    } else {
        info!("[TCP] No backend configured");
    };

    Ok(())
}

/// Helper for creating the relevant HTTP response to write into a `TcpStream`.
fn construct_response(response: Response) -> Result<String, HTTPError> {
    let http_version = response.version();
    let status = response.status();
    let response_body = response.text().map_err(HTTPError::InvalidBody)?;

    let status_line = format!("{:?} {} OK", http_version, status);
    let content_len = format!("Content-Length: {}", response_body.len());

    let response = format!("{status_line}\r\n{content_len}\r\n\r\n{response_body}");

    Ok(response)
}

pub fn http_connection<S>(
    targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    target_name: String,
    routing_idx: Arc<Mutex<usize>>,
    method: String,
    request_path: String,
    stream: S,
) -> Result<()>
where
    S: Read + Write,
    TcpStream: std::convert::From<S>,
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
                let backend_response =
                    client
                        .get(&http_backend)
                        .send()
                        .map_err(|e| HTTPError::ProxyRequest {
                            upstream_address: http_backend.clone(),
                            source: e,
                        })?;
                let response = construct_response(backend_response)?;

                let mut s = TcpStream::from(stream);

                s.write_all(response.as_bytes())?
            }
            "POST" => {
                let backend_response = client.post(&http_backend).send()?;
                let response = construct_response(backend_response)?;

                let mut s = TcpStream::from(stream);

                s.write_all(response.as_bytes())?;
            }
            _ => Err(HTTPError::UnsupportedMethod(method.to_string()))?,
        }
        info!("[HTTP] response sent to {}", &http_backend);
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

    thread::spawn(move || {
        for (name, listener) in bound_listeners {
            for stream in listener.incoming() {
                let name = name.clone();
                let idx = Arc::clone(&idx);
                let current_healthy_targets = Arc::clone(&current_healthy_targets);
                thread::spawn(move || match stream {
                    Ok(mut stream) => {
                        info!("Incoming HTTP request");
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
    });

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
