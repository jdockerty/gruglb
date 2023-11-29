use crate::config::{Backend, Config, Protocol, Target};
use crate::lb::SendTargets;
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::iter::Iterator;
use std::{collections::HashMap, sync::Arc, vec};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

/// Proxy is used to encompass common functionality between L4 and L7 load balancing.
///
/// Traits cannot have `async` functions as part of stable Rust, but this proc-macro
/// makes it possible.
#[async_trait]
pub trait Proxy {
    /// Being accepting a particular type of connection.
    ///
    /// This is limited to TCP-oriented connections, e.g. TCP and HTTP.
    async fn accept(
        listeners: Vec<(String, TcpListener)>,
        current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
    ) -> Result<()>;

    // TODO: add another slash here once impl to stop errors.
    // TODO: think about adding `self` here for connection related info?
    // After accepting an incoming connection for a target, it should be proxied to a healthy backend.
    async fn proxy(mut connection: Connection, routing_idx: Arc<RwLock<usize>>) -> Result<()>;
}

/// Contains useful contextual information about a conducted health check.
/// This is used to aid in updating the condition of backends to be routable.
#[derive(Debug)]
pub struct State {
    pub target_name: String,
    pub backend: Backend,
}

/// Encompassing struct for passing information about a particular connection.
pub struct Connection {
    pub targets: Arc<DashMap<String, Vec<Backend>>>,
    pub stream: TcpStream,
    pub client: Option<Arc<reqwest::Client>>,
    pub target_name: String,
    pub method: Option<String>,
    pub request_path: Option<String>,
}

/// The resulting health check can either be a `Success` or `Failure` mode,
/// depending on the returned response.
#[derive(Debug)]
pub enum Health {
    Success(State),
    Failure(State),
}

/// Conducts health checks against the configured targets.
/// This expects a channel which can be sent to, the receiving end of the channel
/// is used to keep track of whether the backends are routable.
pub async fn health(conf: Arc<Config>, sender: SendTargets) {
    let mut interval = tokio::time::interval(conf.health_check_interval());
    if let Some(targets) = &conf.targets {
        let health_client = reqwest::Client::new();
        loop {
            interval.tick().await;

            let mut results: Vec<Health> = vec![];

            for (name, target) in targets {
                match target.protocol_type() {
                    Protocol::Http => {
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

                                        results.push(Health::Success(State {
                                            target_name: name.clone(),
                                            backend: backend.clone(),
                                        }));
                                    }
                                    Err(_) => {
                                        info!("({name}, {request_addr}) is unhealthy, removing from pool");
                                        results.push(Health::Failure(State {
                                            target_name: name.clone(),
                                            backend: backend.clone(),
                                        }));
                                        info!("[HTTP] Sending failure to channel");
                                    }
                                }
                            }
                        } else {
                            info!("No backends to health check for {}", name);
                        }
                    }
                    Protocol::Tcp => {
                        if let Some(backends) = &target.backends {
                            for backend in backends {
                                let request_addr = &format!("{}:{}", backend.host, backend.port);

                                if let Ok(mut stream) = TcpStream::connect(request_addr).await {
                                    info!("{request_addr} is healthy backend for {}", name);
                                    stream.shutdown().await.unwrap();
                                    results.push(Health::Success(State {
                                        target_name: name.clone(),
                                        backend: backend.clone(),
                                    }))
                                } else {
                                    info!(
                                        "({name}, {request_addr}) is unhealthy, removing from pool"
                                    );
                                    results.push(Health::Failure(State {
                                        target_name: name.clone(),
                                        backend: backend.clone(),
                                    }))
                                    // This is "removed from the pool" because it is not included in
                                    // the vector for the next channel transmission, so traffic does not get routed
                                    // to it.
                                }
                            }
                        } else {
                            info!("No backends to health check for {}", name);
                        }
                    }
                    Protocol::Unsupported => {
                        error!("({name}, {target:?}) sent an unsupported protocol, cannot perform health check.");
                    }
                }
            }
            sender.send(results).await.unwrap();
        }
    } else {
        info!("No targets configured, cannot perform health checks.");
    }
}

/// Proxy a TCP connection to a range of configured backend servers.
pub async fn tcp_connection(
    targets: Arc<DashMap<String, Vec<Backend>>>,
    target_name: String,
    routing_idx: Arc<RwLock<usize>>,
    mut stream: TcpStream,
) -> Result<()> {
    if let Some(backends) = targets.get(&target_name) {
        let backends = backends.to_vec();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        if backend_count == 0 {
            info!("[TCP] No routable backends for {target_name}, nothing to do");
            return Ok(());
        }

        // Limit the scope of the index write lock.
        let backend_addr: String;
        {
            let mut idx = routing_idx.write().await;
            debug!(
                "[TCP] {backend_count} backends configured for {target_name}, current index {idx}"
            );

            // Reset index when out of bounds to route back to the first server.
            if *idx >= backend_count {
                *idx = 0;
            }

            backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);

            // Increment a shared index after we've constructed our current connection
            // address.
            *idx += 1;
        }

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

/// Accept TCP connections by binding to multiple `TcpListener` socket address and
/// handling incoming connections, passing them to the configured TCP backends.
pub async fn accept_tcp(
    bind_address: String,
    current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
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
