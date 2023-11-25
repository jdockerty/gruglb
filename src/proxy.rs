use crate::config::{Backend, Config, Protocol, Target};
use crate::lb::SendTargets;
use anyhow::{Context, Result};
use reqwest::Response;
use std::iter::Iterator;
use std::{collections::HashMap, sync::Arc, thread, time::Duration, vec};
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

fn health_check_wait(d: Duration) {
    thread::sleep(d);
}

#[derive(Debug)]
pub struct CheckState {
    pub target_name: String,
    pub backend: Backend,
}

#[derive(Debug)]
pub enum Health {
    Success(CheckState),
    Failure(CheckState),
}

pub async fn http_health(conf: Arc<Config>, sender: SendTargets) {
    let health_client = reqwest::Client::new();

    if let Some(targets) = &conf.targets {
        loop {
            info!("Starting HTTP health checks");
            for (name, target) in targets {
                if target.protocol_type() != Protocol::Http {
                    continue;
                }

                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!(
                            "http://{}:{}{}",
                            backend.host,
                            backend.port,
                            backend.health_path.clone().unwrap()
                        );

                        match health_client.get(request_addr).send().await {
                            Ok(_response) => {
                                info!("{request_addr} is healthy backend for {}", name);
                                info!("[HTTP] Sending success to channel");

                                sender
                                    .send(Health::Success(CheckState {
                                        target_name: name.to_string(),
                                        backend: backend.clone(),
                                    }))
                                    .await
                                    .unwrap();
                            }
                            Err(_) => {
                                info!("({name}, {request_addr}) is unhealthy, removing from pool");
                                info!("[HTTP] Sending failure to channel");
                                sender
                                    .send(Health::Failure(CheckState {
                                        target_name: name.to_string(),
                                        backend: backend.clone(),
                                    }))
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                } else {
                    info!("No backends to health check for {}", name);
                }
            }
            health_check_wait(conf.health_check_interval());
        }
    } else {
        info!("[HTTP] No targets configured, unable to health check.");
    }
}

/// Run health checks against the configured TCP targets.
pub async fn tcp_health(conf: Arc<Config>, sender: SendTargets) {
    if let Some(targets) = &conf.targets {
        loop {
            info!("Starting TCP health checks");
            for (name, target) in targets {
                if target.protocol_type() != Protocol::Tcp {
                    continue;
                }
                // healthy_targets.insert(name.to_string(), healthy_backends.clone());
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!("{}:{}", backend.host, backend.port);

                        if let Ok(mut stream) = TcpStream::connect(request_addr).await {
                            stream.shutdown().await.unwrap();
                            info!("{request_addr} is healthy backend for {}", name);
                            sender
                                .send(Health::Success(CheckState {
                                    target_name: name.to_string(),
                                    backend: backend.clone(),
                                }))
                                .await
                                .unwrap();
                            // healthy_backends.push(backend.clone());
                        } else {
                            info!("({name}, {request_addr}) is unhealthy, removing from pool");
                            // This is "removed from the pool" because it is not included in
                            // the vector for the next channel transmission, so traffic does not get routed
                            // to it.
                            sender
                                .send(Health::Failure(CheckState {
                                    target_name: name.to_string(),
                                    backend: backend.clone(),
                                }))
                                .await
                                .unwrap();
                        }
                    }
                } else {
                    info!("No backends to health check for {}", name);
                }
            }
            info!("[TCP] Sending targets to channel");
            health_check_wait(conf.health_check_interval());
        }
    } else {
        info!("[TCP] No targets configured, unable to health check.");
    }
}

// Proxy a TCP connection to a range of configured backend servers.
pub async fn tcp_connection(
    targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    target_name: String,
    routing_idx: Arc<RwLock<usize>>,
    mut stream: TcpStream,
) -> Result<()> {
    if let Some(backends) = targets.read().await.get(&target_name) {
        let backends = backends.to_vec();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        if backend_count == 0 {
            info!("[TCP] No routable backends for {target_name}, nothing to do");
            return Ok(());
        }

        let mut idx = routing_idx.write().await;

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

        let mut response = TcpStream::connect(&backend_addr).await?;
        let mut buffer = Vec::new();
        response.read_to_end(&mut buffer).await?;
        stream.write_all(&buffer).await?;
        debug!("TCP stream closed");
    } else {
        info!("[TCP] No backend configured");
    };

    Ok(())
}

/// Helper for creating the relevant HTTP response to write into a `TcpStream`.
async fn construct_response(response: Response) -> Result<String> {
    let http_version = response.version();
    let status = response.status();
    let response_body = response.text().await?;
    let status_line = format!("{:?} {} OK", http_version, status);
    let content_len = format!("Content-Length: {}", response_body.len());

    let response = format!("{status_line}\r\n{content_len}\r\n\r\n{response_body}");

    Ok(response)
}

pub async fn http_connection(
    client: Arc<reqwest::Client>,
    targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    target_name: String,
    routing_idx: Arc<RwLock<usize>>,
    method: String,
    request_path: String,
    mut stream: TcpStream,
) -> Result<()> {
    if let Some(backends) = targets.read().await.get(&target_name) {
        let backends = backends.to_vec();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        if backend_count == 0 {
            info!("[HTTP] No routable backends for {target_name}, nothing to do");
            return Ok(());
        }

        let mut idx = routing_idx.write().await;

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

        match method.as_str() {
            "GET" => {
                let backend_response = client
                    .get(&http_backend)
                    .send()
                    .await
                    .with_context(|| format!("unable to send response to {http_backend}"))?;
                let response = construct_response(backend_response).await?;

                stream.write_all(response.as_bytes()).await?;
            }
            "POST" => {
                let backend_response = client
                    .post(&http_backend)
                    .send()
                    .await
                    .with_context(|| format!("unable to send response to {http_backend}"))?;
                let response = construct_response(backend_response).await?;

                stream.write_all(response.as_bytes()).await?;
            }
            _ => {
                error!("Unsupported: {method}")
            }
        }
        info!("[HTTP] response sent to {}", &http_backend);
    } else {
        info!("[HTTP] No backend configured");
    };

    Ok(())
}

/// Bind to the configured listener ports for incoming TCP connections.
async fn generate_tcp_listeners(
    bind_address: String,
    targets: HashMap<String, Target>,
) -> Result<Vec<(String, TcpListener)>> {
    let mut tcp_bindings = vec![];

    for (name, target) in targets {
        if target.protocol_type() == Protocol::Tcp {
            let addr = format!("{}:{}", bind_address.clone(), target.listener.unwrap());
            let listener = TcpListener::bind(&addr).await?;
            info!("Binding to {} for {}", &addr, &name);
            tcp_bindings.push((name, listener));
        }
    }

    Ok(tcp_bindings)
}

async fn generate_http_listeners(
    bind_address: String,
    targets: HashMap<String, Target>,
) -> Result<Vec<(String, TcpListener)>> {
    let mut http_bindings = vec![];

    for (name, target) in targets {
        if target.protocol_type() == Protocol::Http {
            let addr = format!("{}:{}", bind_address.clone(), target.listener.unwrap());
            let listener = TcpListener::bind(&addr).await?;
            info!("Binding to {} for {}", &addr, &name);
            http_bindings.push((name, listener));
        }
    }

    Ok(http_bindings)
}

pub async fn accept_http(
    client: Arc<reqwest::Client>,
    bind_address: String,
    current_healthy_targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    targets: HashMap<String, Target>,
) -> Result<()> {
    let idx: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
    let bound_listeners = generate_http_listeners(bind_address, targets).await?;

    tokio::spawn(async move {
        for (name, listener) in bound_listeners {
            while let Ok((mut stream, address)) = listener.accept().await {
                let name = name.clone();
                let idx = Arc::clone(&idx);
                let current_healthy_targets = Arc::clone(&current_healthy_targets);
                info!("Incoming HTTP request from {address}");
                let buf = BufReader::new(&mut stream);
                let mut lines = buf.lines();
                let mut http_request: Vec<String> = vec![];

                while let Some(line) = lines.next_line().await.unwrap() {
                    if line.is_empty() {
                        break;
                    }
                    http_request.push(line);
                }

                let info = http_request[0].clone();
                let http_info = info
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();

                let method = http_info[0].clone();
                let path = http_info[1].clone();
                let client = client.clone();
                tokio::spawn(async move {
                    info!("{method} request at {path}");
                    http_connection(
                        client,
                        current_healthy_targets,
                        name,
                        idx,
                        method.to_string(),
                        path.to_string(),
                        stream,
                    )
                    .await
                    .unwrap();
                });
            }
        }
    });

    Ok(())
}

/// Accept TCP connections by binding to multiple `TcpListener` socket address and
/// handling incoming connections, passing them to the configured TCP backends.
pub async fn accept_tcp(
    bind_address: String,
    current_healthy_targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    targets: HashMap<String, Target>,
) -> Result<()> {
    let idx: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
    let bound_listeners = generate_tcp_listeners(bind_address, targets).await?;

    for (name, listener) in bound_listeners {
        // Listen to incoming traffic on separate threads
        let idx = Arc::clone(&idx);
        let current_healthy_targets = Arc::clone(&current_healthy_targets);

        tokio::spawn(async move {
            while let Ok((stream, remote_peer)) = listener.accept().await {
                info!("Incoming request on {remote_peer}");

                let idx = Arc::clone(&idx);
                let tcp_targets = Arc::clone(&current_healthy_targets);
                // Pass the TCP streams over to separate threads to avoid
                // blocking and give each thread its copy of the configuration.
                let target_name = name.clone();

                tokio::spawn(async move {
                    tcp_connection(tcp_targets, target_name, idx, stream)
                        .await
                        .unwrap();
                })
                .await
                .unwrap();
            }
        });
    }

    Ok(())
}
