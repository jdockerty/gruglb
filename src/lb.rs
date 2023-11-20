use crate::config::{Backend, Config};
use crate::proxy;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::task;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

pub type SendTargets = Sender<HashMap<String, Vec<Backend>>>;
pub type RecvTargets = Receiver<HashMap<String, Vec<Backend>>>;

/// Load balancer application
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
            info!("Receiving healthy targets");
            loop {
                for (target, backends) in receiver.recv().await.unwrap() {
                    healthy_targets.write().await.insert(target, backends);
                }
                thread::sleep(Duration::from_millis(500));
            }
        });

        proxy::accept_tcp(
            self.conf
                .address
                .clone()
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            Arc::clone(&self.current_healthy_targets),
            self.conf.targets.clone().unwrap(),
        )
        .await?;

        //proxy::accept_http(
        //    self.conf
        //        .address
        //        .clone()
        //        .unwrap_or_else(|| "127.0.0.1".to_string()),
        //    Arc::clone(&self.current_healthy_targets),
        //    self.conf.targets.clone().unwrap(),
        //)
        //.await?;

        Ok(())
    }
}
