use crate::config::Protocol;
use crate::config::{Backend, Config};
use crate::http::HttpProxy;
use crate::proxy::{self, Health};
use crate::tcp::TcpProxy;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task;
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
            while let Some(results) = receiver.recv().await {
                if health_recv_token.is_cancelled() {
                    info!("[CANCEL] Running final health check operation");
                    receiver.close();
                    while let Some(final_results) = receiver.recv().await {
                        LB::handle_health_results(final_results, healthy_targets.clone());
                    }
                    break;
                }
                LB::handle_health_results(results, healthy_targets.clone());
            }
        });

        let tcp_listeners = self
            .generate_listeners(
                self.conf
                    .address
                    .clone()
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
                self.conf.targets.clone().unwrap(),
                Protocol::Tcp,
            )
            .await?;
        let http_listeners = self
            .generate_listeners(
                self.conf
                    .address
                    .clone()
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
                self.conf.targets.clone().unwrap(),
                Protocol::Http,
            )
            .await?;

        if self.conf.requires_proxy_type(Protocol::Tcp) {
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
        bind_address: String,
        targets: std::collections::HashMap<String, crate::config::Target>,
        protocol_type: Protocol,
    ) -> Result<Vec<(String, TcpListener)>> {
        let mut bindings = vec![];

        for (name, target) in targets {
            if target.protocol_type() == protocol_type {
                let addr = format!("{}:{}", bind_address.clone(), target.listener.unwrap());
                let listener = TcpListener::bind(&addr).await?;
                info!("Binding to {} for {}", &addr, &name);
                bindings.push((name, listener));
            }
        }

        Ok(bindings)
    }
}
