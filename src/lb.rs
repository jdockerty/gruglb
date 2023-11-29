use crate::config::Protocol;
use crate::config::{Backend, Config};
use crate::http::HttpProxy;
use crate::proxy::{self, Health, Proxy};
use crate::tcp::TcpProxy;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

pub type SendTargets = Sender<Vec<Health>>;
pub type RecvTargets = Receiver<Vec<Health>>;

/// Load balancer application
#[derive(Debug)]
pub struct LB {
    pub conf: Arc<Config>,
    pub current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
}

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

impl LB {
    /// Run the application.
    pub async fn run(&self, sender: SendTargets, mut receiver: RecvTargets) -> Result<()> {
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
        task::spawn(async move {
            while let Some(results) = receiver.recv().await {
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

        info!("Accepting tcp!");
        TcpProxy::accept(tcp_listeners, Arc::clone(&self.current_healthy_targets)).await?;

        info!("Accepting http!");
        HttpProxy::accept(http_listeners, Arc::clone(&self.current_healthy_targets)).await?;
        Ok(())
    }

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
