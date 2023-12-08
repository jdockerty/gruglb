use crate::config::{Backend, Config, Protocol, TLSConfig};
use crate::lb::{ListenerConfig, SendTargets};
use anyhow::{Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use std::iter::Iterator;
use std::{sync::Arc, vec};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
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
    /// Proxy a `TcpStream` from an incoming connection to configured targets, with accompanying
    /// `Connection` related data.
    async fn proxy(
        &'static self,
        mut connection: Connection,
        routing_idx: Arc<RwLock<usize>>,
    ) -> Result<()>;

    /// Retrieve the type of protocol in use by the current proxy.
    fn protocol_type(&self) -> Protocol;
}

/// Accepting a particular type of connection for configured listeners and targets.
///
/// This is limited to TCP-oriented connections, e.g. TCP and HTTP(S).
pub async fn accept<P>(
    proxy: &'static P,
    listeners: Vec<ListenerConfig>,
    current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
    cancel: CancellationToken,
) -> Result<()>
where
    P: Proxy + Send + Sync,
{
    let idx: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
    for conf in listeners {
        let idx = idx.clone();

        let client = Arc::new(reqwest::Client::new());
        // use native_tls instead of reqwest embed.
        //let client = match conf.tls {
        //    Some(tls) => {
        //        info!("Loading TLS info");
        //        let mut pem = vec![];
        //        let mut cert_file = File::open(tls.cert_file.clone()).await.with_context(|| format!("unable to open certificate file: {}", tls.clone().cert_file.display()))?;
        //        let mut key_file = File::open(tls.cert_key.clone()).await.with_context(|| format!("unable to open key file: {}", tls.clone().cert_key.display()))?;
        //        cert_file.read_to_end(&mut pem).await?;
        //        key_file.read_to_end(&mut pem).await?;

        //        let id = Identity::from_pem(&pem)?;

        //        let client = reqwest::ClientBuilder::new()
        //            .identity(id)
        //            .build()?;
        //        Arc::new(client)
        //    }
        //    None => Arc::new(reqwest::Client::new()),
        //};
        let current_healthy_targets = current_healthy_targets.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, address)) = conf.listener.accept().await {
                if cancel.is_cancelled() {
                    info!(
                        "[CANCEL] Received cancel, no longer receiving any {} requests.",
                        proxy.protocol_type()
                    );
                    stream.shutdown().await.unwrap();
                    break;
                }
                let name = conf.target_name.clone();
                let idx = Arc::clone(&idx);
                let current_healthy_targets = Arc::clone(&current_healthy_targets);
                info!(
                    "[{}] Incoming request from {address}",
                    proxy.protocol_type()
                );
                let client = client.clone();
                tokio::spawn(async move {
                    // debug!("{method} request at {path}");
                    let connection = Connection {
                        client: Some(client),
                        targets: current_healthy_targets,
                        target_name: name,
                        stream,
                    };
                    proxy.proxy(connection, idx).await.unwrap();
                });
            }
        });
    }
    Ok(())
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

async fn http_health(
    results: &mut Vec<Health>,
    name: String,
    backends: &Vec<Backend>,
    health_client: reqwest::Client,
) -> Result<()> {
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
                info!("[HTTP] Pushing success");

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
                info!("[HTTP] Pushing failure");
            }
        }
    }

    Ok(())
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
                            http_health(
                                &mut results,
                                name.clone(),
                                backends,
                                health_client.clone(),
                            )
                            .await
                            .unwrap();
                        } else {
                            info!("No backends to health check for {}", name);
                        }
                    }
                    Protocol::Https => {
                        if let Some(backends) = &target.backends {
                            http_health(
                                &mut results,
                                name.clone(),
                                backends,
                                health_client.clone(),
                            )
                            .await
                            .unwrap();
                        } else {
                            info!("No backends to health check for {}", name);
                        }
                    }
                    Protocol::Tcp => {
                        if let Some(backends) = &target.backends {
                            for backend in backends {
                                let request_addr = &format!("{}:{}", backend.host, backend.port);

                                if let Ok(mut stream) = TcpStream::connect(request_addr).await {
                                    stream.shutdown().await.unwrap();
                                    info!("{request_addr} is healthy backend for {}", name);
                                    info!("[TCP] Pushing success");
                                    results.push(Health::Success(State {
                                        target_name: name.clone(),
                                        backend: backend.clone(),
                                    }))
                                } else {
                                    info!(
                                        "({name}, {request_addr}) is unhealthy, removing from pool"
                                    );
                                    info!("[TCP] Pushing failure");
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
            info!("Sending targets to channel");
            sender.send(results).await.unwrap();
        }
    } else {
        info!("No targets configured, cannot perform health checks.");
    }
}
