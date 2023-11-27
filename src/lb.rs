use crate::config::{Backend, Config};
use crate::proxy::{self, Health};
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
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
        // Initialise the targets to avoid deadlocks on startup.
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
                                info!("Backend not in pool, adding");
                                backends.push(backend);
                            } else {
                                info!("Backend exists in healthy state, nothing to do");
                            }
                        }
                        Health::Failure(state) => {
                            let target_name = state.target_name.clone();
                            let backend = state.backend.clone();

                            // Default is a Vec<Backend>, so inserts an empty Vec if not present.
                            let mut backends = healthy_targets.entry(target_name).or_default();

                            if let Some(idx) = backends.iter().position(|b| b == &backend) {
                                info!("Backend exists in healthy state, removing from pool");
                                backends.remove(idx);
                            } else {
                                info!("Backend not in pool, nothing to do");
                            }
                        }
                    }
                }
            }
        });

        info!("Accepting tcp!");
        proxy::accept_tcp(
            self.conf
                .address
                .clone()
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            Arc::clone(&self.current_healthy_targets),
            self.conf.targets.clone().unwrap(),
        )
        .await?;

        info!("Accepting http!");
        let http_client = Arc::new(reqwest::Client::new());
        proxy::accept_http(
            http_client,
            self.conf
                .address
                .clone()
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            Arc::clone(&self.current_healthy_targets),
            self.conf.targets.clone().unwrap(),
        )
        .await?;

        Ok(())
    }
}
