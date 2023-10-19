use crate::{config, proxy};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Arc, RwLock,
};
use tracing::{debug, error, info};
use tracing_subscriber::FmtSubscriber;

type SendTargets = Sender<HashMap<String, Vec<config::Backend>>>;
type RecvTargets = Receiver<HashMap<String, Vec<config::Backend>>>;

/// Load balancer application
struct LB {
    pub conf: Arc<config::Config>,
    current_healthy_targets: Arc<RwLock<HashMap<String, Vec<config::Backend>>>>,
    sender: SendTargets,
    receiver: RecvTargets,
}

/// Construct a new instance of gruglb
pub fn new(conf: config::Config, logging: bool) -> LB {
    let (tx, rx): (SendTargets, RecvTargets) = channel();

    if logging {
        FmtSubscriber::builder()
            .with_max_level(conf.log_level())
            .init();
    }
    return LB {
        sender: tx,
        receiver: rx,
        conf: Arc::new(conf),
        current_healthy_targets: Arc::new(RwLock::new(HashMap::new())),
    };
}

impl LB {
    pub fn run(
        &self,
        conf: Arc<Config>,
        targets: HashMap<String, Target>,
        sender: SendTargets,
        receiver: RecvTargets,
    ) -> Result<()> {
        if let Some(target_names) = conf.target_names() {
            debug!("All loaded targets {:?}", target_names);
        }

        // Provides the health check thread with its own configuration.
        let health_check_conf = conf.clone();
        thread::spawn(move || tcp_health(health_check_conf, sender));
        let healthy_targets = Arc::clone(&TCP_CURRENT_HEALTHY_TARGETS);

        // Continually receive from the channel and update our healthy backend state.
        thread::spawn(move || loop {
            for (target, backends) in receiver.recv().unwrap() {
                healthy_targets.write().unwrap().insert(target, backends);
            }
            thread::sleep(Duration::from_secs(2));
        });

        bind_tcp_listeners(conf, targets)?;

        Ok(())
    }
}
