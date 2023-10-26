use crate::config::{Backend, Config};
use crate::proxy;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::mpsc::SyncSender;
use std::sync::{mpsc::Receiver, Arc, RwLock};
use std::thread;
use std::time::Duration;
use tracing::debug;
use tracing_subscriber::FmtSubscriber;

pub type SendTargets = SyncSender<HashMap<String, Vec<Backend>>>;
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
    pub fn run(&self, sender: SendTargets, receiver: RecvTargets) -> Result<()> {
        if let Some(target_names) = self.conf.target_names() {
            debug!("All loaded targets {:?}", target_names);
        }

        // Provides the health check thread with its own configuration.
        let health_check_conf = self.conf.clone();
        thread::spawn(move || proxy::tcp_health(health_check_conf, sender));
        let healthy_targets = Arc::clone(&self.current_healthy_targets);

        // Continually receive from the channel and update our healthy backend state.
        thread::spawn(move || loop {
            for (target, backends) in receiver.recv().unwrap() {
                healthy_targets.write().unwrap().insert(target, backends);
            }
            thread::sleep(Duration::from_secs(2));
        });

        proxy::bind_tcp_listeners(
            self.conf
                .address
                .clone()
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            Arc::clone(&self.current_healthy_targets),
            self.conf.targets.clone().unwrap(),
        )?;

        Ok(())
    }
}
