use crate::config::Protocol;
use crate::config::{Backend, Config};
use crate::http::HttpProxy;
use crate::https::HttpsProxy;
use crate::proxy::{self, Health};
use crate::tcp::TcpProxy;
use anyhow::Context;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task;
use tokio_native_tls::{
    native_tls::{Identity, TlsAcceptor as SyncTlsAcceptor},
    TlsAcceptor,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use tracing_subscriber::FmtSubscriber;

pub type SendTargets = Sender<Vec<Health>>;
pub type RecvTargets = Receiver<Vec<Health>>;

/// Load balancer application
#[derive(Debug)]
pub struct LB {
    pub conf: Arc<Config>,
    pub current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
}

/// Helper to encompass general listener information.
pub struct ListenerConfig {
    pub target_name: String,
    pub listener: TcpListener,
    pub tls: Option<TlsAcceptor>,
}

impl Drop for LB {
    fn drop(&mut self) {
        info!("Load balancer shut down, all operations completed.");
        if let Some(targets) = self.conf.targets.clone() {
            debug!("Configured targets:");
            for (name, target) in targets {
                debug!("{name}:{target:?}");
            }
        }
    }
}

impl LB {
    /// Construct a new instance of gruglb
    pub fn new(conf: Config) -> LB {
        let _ = FmtSubscriber::builder()
            .with_max_level(conf.log_level())
            .try_init();

        LB {
            conf: Arc::new(conf),
            current_healthy_targets: Arc::new(DashMap::new()),
        }
    }
    /// Run the application.
    pub async fn run(
        &self,
        sender: SendTargets,
        mut receiver: RecvTargets,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        if let Some(target_names) = self.conf.target_names() {
            debug!("All loaded targets {:?}", target_names);
        }

        // Provides the health check thread with its own configuration.
        let health_conf = self.conf.clone();
        task::spawn(async move {
            proxy::health(health_conf, sender).await;
        });

        let healthy_targets = Arc::clone(&self.current_healthy_targets);

        // Continually receive from the channel and update our healthy backend state.
        let health_recv_token = cancel_token.clone();
        task::spawn(async move {
            loop {
                match receiver.try_recv() {
                    Ok(results) => {
                        if health_recv_token.is_cancelled() {
                            info!("[CANCEL] Running final health check operation");
                            receiver.close();
                            while let Some(final_results) = receiver.recv().await {
                                LB::handle_health_results(final_results, healthy_targets.clone());
                            }
                            return;
                        }
                        LB::handle_health_results(results, healthy_targets.clone());
                    }
                    Err(TryRecvError::Empty) => {
                        debug!("Empty, can receive!");
                        // Pause for a static time to avoid overloading the Empty branch
                        // when no messages have been sent, such as in a lengthy health
                        // check duration.
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    Err(TryRecvError::Disconnected) => {
                        error!("Tried to read from channel after cancel was received from sender!")
                    }
                }
            }
        });

        let bind_address = self
            .conf
            .address
            .clone()
            .unwrap_or_else(|| "127.0.0.1".to_string());

        if self.conf.requires_proxy_type(Protocol::Tcp) {
            let tcp_listeners = self
                .generate_listeners(
                    &bind_address,
                    self.conf.targets.clone().unwrap(),
                    Protocol::Tcp,
                )
                .await?;
            info!("Starting TCP!");
            let tcp_cancel_token = cancel_token.clone();
            let tcp = TcpProxy::new();
            proxy::accept(
                tcp,
                tcp_listeners,
                Arc::clone(&self.current_healthy_targets),
                tcp_cancel_token,
            )
            .await?;
        }

        if self.conf.requires_proxy_type(Protocol::Http) {
            let http_listeners = self
                .generate_listeners(
                    &bind_address,
                    self.conf.targets.clone().unwrap(),
                    Protocol::Http,
                )
                .await?;
            info!("Starting HTTP!");
            let http_cancel_token = cancel_token.clone();
            let http = HttpProxy::new();
            proxy::accept(
                http,
                http_listeners,
                Arc::clone(&self.current_healthy_targets),
                http_cancel_token,
            )
            .await?;
        }

        if self.conf.requires_proxy_type(Protocol::Https) {
            let https_listeners = self
                .generate_listeners(
                    &bind_address,
                    self.conf.targets.clone().unwrap(),
                    Protocol::Https,
                )
                .await?;
            info!("Starting HTTPS!");
            let http_cancel_token = cancel_token.clone();
            let https = HttpsProxy::new();
            proxy::accept(
                https,
                https_listeners,
                Arc::clone(&self.current_healthy_targets),
                http_cancel_token,
            )
            .await?;
        }

        Ok(())
    }

    /// Utility function to handle the health check results.
    fn handle_health_results(
        results: Vec<Health>,
        healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
    ) {
        for health_result in results {
            match health_result {
                Health::Success(state) => {
                    let target_name = state.target_name.clone();
                    let backend = state.backend.clone();

                    // Default is a Vec<Backend>, so inserts an empty Vec if not present.
                    let mut backends = healthy_targets.entry(target_name).or_default();

                    if !backends.iter().any(|b| b == &backend) {
                        info!("({}, {}) not in pool, adding", state.target_name, backend);
                        backends.push(backend);
                    } else {
                        info!(
                            "({}, {}) exists in healthy state, nothing to do",
                            state.target_name, backend
                        );
                    }
                }
                Health::Failure(state) => {
                    let target_name = state.target_name.clone();
                    let backend = state.backend.clone();

                    // Default is a Vec<Backend>, so inserts an empty Vec if not present.
                    let mut backends = healthy_targets.entry(target_name).or_default();

                    if let Some(idx) = backends.iter().position(|b| b == &backend) {
                        info!(
                            "({}, {}) is unhealthy, removing from pool",
                            state.target_name, backend
                        );
                        backends.remove(idx);
                    } else {
                        info!(
                            "({}, {}) still unhealthy, nothing to do",
                            state.target_name, backend
                        );
                    }
                }
            }
        }
    }

    /// Generate `TcpListener`'s for a `protocol_type`.
    ///
    /// Currently only `Protocol::Tcp` and `Protocol::Http` are supported.
    async fn generate_listeners(
        &self,
        bind_address: &str,
        targets: std::collections::HashMap<String, crate::config::Target>,
        protocol_type: Protocol,
    ) -> Result<Vec<ListenerConfig>> {
        let mut bindings = vec![];

        for (name, target) in targets {
            if target.protocol_type() == protocol_type {
                let addr = format!("{}:{}", bind_address.to_owned(), target.listener.unwrap());
                let listener = TcpListener::bind(&addr).await?;
                info!("Binding to {} for {}", &addr, &name);

                let tls = if protocol_type == Protocol::Https {
                    // This is a HTTPS target, so it should have a TLSConfig.
                    let target_tls = target.clone().tls.unwrap();
                    let mut pem = vec![];
                    let mut key = vec![];

                    let mut cert_file = File::open(target_tls.cert_file.clone())
                        .await
                        .with_context(|| {
                            format!(
                                "unable to open certificate file: {}",
                                target_tls.clone().cert_file.display()
                            )
                        })?;
                    let mut key_file =
                        File::open(target_tls.cert_key.clone())
                            .await
                            .with_context(|| {
                                format!(
                                    "unable to open key file: {}",
                                    target_tls.clone().cert_key.display()
                                )
                            })?;
                    cert_file.read_to_end(&mut pem).await?;
                    key_file.read_to_end(&mut key).await?;
                    let native_acceptor = SyncTlsAcceptor::new(Identity::from_pkcs8(&pem, &key)?)?;
                    let acceptor = TlsAcceptor::from(native_acceptor);

                    Some(acceptor)
                } else {
                    None
                };

                let conf = ListenerConfig {
                    target_name: name,
                    listener,
                    tls,
                };
                bindings.push(conf);
            }
        }

        Ok(bindings)
    }
}
