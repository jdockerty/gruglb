use crate::config::{Backend, Config};
use crate::proxy;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::task;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

pub type SendTargets = Sender<HashMap<String, Vec<Backend>>>;
pub type RecvTargets = Receiver<HashMap<String, Vec<Backend>>>;

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
        task::spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(msg) => {
                        for target_name in msg.keys() {
                            let backends = msg.get(target_name).unwrap().to_vec();
                            if let Some(old) = healthy_targets
                                .write()
                                .await
                                .insert(target_name.to_string(), backends.clone())
                            {
                                if old == backends {
                                    info!("Backends for {target_name} unchanged.");
                                    continue;
                                }
                                info!("Updated {target_name} from {old:?} to {backends:?}");
                            } else {
                                info!("Updated {target_name} with {backends:?}");
                            }
                        }
                    }
                    None => info!("Nothing to do, waiting for next receive cycle."),
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
