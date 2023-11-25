use crate::config::{Backend, Config};
use crate::proxy::{self, Health};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::task;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

pub type SendTargets = Sender<Health>;
pub type RecvTargets = Receiver<Health>;

/// Load balancer application
#[derive(Debug)]
pub struct LB {
    pub conf: Arc<Config>,
    pub current_healthy_targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
}

/// Construct a new instance of gruglb
pub fn new(conf: Config) -> LB {
    let _ = FmtSubscriber::builder()
        .with_max_level(conf.log_level())
        .try_init();

    LB {
        conf: Arc::new(conf),
        current_healthy_targets: Arc::new(RwLock::new(HashMap::new())),
    }
}

impl LB {
    /// Run the application.
    pub async fn run(&self, sender: SendTargets, mut receiver: RecvTargets) -> Result<()> {
        if let Some(target_names) = self.conf.target_names() {
            debug!("All loaded targets {:?}", target_names);
        }

        // Provides the health check thread with its own configuration.
        let tcp_conf = self.conf.clone();
        let http_conf = self.conf.clone();
        let tcp_sender = sender.clone();
        let http_sender = sender.clone();
        task::spawn(async move {
            proxy::tcp_health(tcp_conf, tcp_sender).await;
        });
        task::spawn(async move {
            proxy::http_health(http_conf, http_sender).await;
        });
        let healthy_targets = Arc::clone(&self.current_healthy_targets);

        // Continually receive from the channel and update our healthy backend state.
        // Initialise the targets to avoid deadlocks on startup.
        let init_conf = self.conf.clone();
        task::spawn(async move {
            info!("Initialising targets");
            for k in init_conf.targets.clone().unwrap().keys() {
                healthy_targets.write().await.insert(k.to_string(), vec![]);
            }

            while let Some(msg) = receiver.recv().await {
                match msg {
                    Health::Success(state) => {
                        if let Some(backends) = healthy_targets.read().await.get(&state.target_name)
                        {
                            let mut backends = backends.to_vec();
                            if let Some(_idx) = backends.iter().position(|b| b == &state.backend) {
                                info!(
                                    "Backend for {} already healthy, nothing to do",
                                    state.target_name
                                );
                            } else {
                                backends.push(state.backend);

                                if let Ok(mut healthy_targets) = healthy_targets.try_write() {
                                    healthy_targets.insert(state.target_name, backends);
                                } else {
                                    info!(
                                        "Unable to acquire write lock for {}, moving to next cycle",
                                        state.target_name
                                    );
                                    tokio::time::sleep(tokio::time::Duration::from_millis(250))
                                        .await;
                                }
                            };
                        }
                    }
                    Health::Failure(state) => {
                        if let Some(backends) = healthy_targets.read().await.get(&state.target_name)
                        {
                            let mut backends = backends.to_vec();
                            if let Some(idx) = backends.iter().position(|b| b == &state.backend) {
                                let _ = backends.remove(idx);
                                info!("Updating {} with {:?}", state.target_name, backends);
                                if let Ok(mut healthy_targets) = healthy_targets.try_write() {
                                    healthy_targets.insert(state.target_name, backends);
                                } else {
                                    info!(
                                        "Unable to acquire write lock for {}, moving to next cycle",
                                        state.target_name
                                    );
                                    tokio::time::sleep(tokio::time::Duration::from_millis(250))
                                        .await;
                                    continue;
                                }
                            } else {
                                info!("{:?} is not in healthy target mapping for {}, nothing to remove", state.backend, state.target_name);
                            };
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
