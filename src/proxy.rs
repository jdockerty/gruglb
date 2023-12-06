use crate::config::{Backend, Config, Protocol};
use crate::lb::SendTargets;
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::iter::Iterator;
use std::{sync::Arc, vec};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// Proxy is used to encompass common functionality between L4 and L7 load balancing.
///
/// Traits cannot have `async` functions as part of stable Rust, but this proc-macro
/// makes it possible.
#[async_trait]
pub trait Proxy {
    /// Accepting a particular type of connection for configured listeners and targets.
    ///
    /// This is limited to TCP-oriented connections, e.g. TCP and HTTP.
    async fn accept(
        &'static self,
        listeners: Vec<(String, TcpListener)>,
        current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
        cancel: CancellationToken,
    ) -> Result<()>;

    /// Proxy a `TcpStream` from an incoming connection to configured targets, with accompanying
    /// `Connection` related data.
    async fn proxy(
        &'static self,
        mut connection: Connection,
        routing_idx: Arc<RwLock<usize>>,
    ) -> Result<()>;
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
    pub target_name: String,

    /// An optional HTTP client used to make requests.
    pub client: Option<Arc<reqwest::Client>>,
    // An optional HTTP `method` for the connection, for example `GET`.
    // This is only used for HTTP connections.
    // pub method: Option<String>,

    // An optional `request_path` for the connection, for example `/api/v1/users`.
    // This is only used for HTTP connections.
    // pub request_path: Option<String>,
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

            if sender.is_closed() {
                info!("[CANCEL] No more health checks will be sent.");
                break;
            }

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
