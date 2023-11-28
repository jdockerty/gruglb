use crate::config::Backend;
use crate::config::Protocol;
use crate::config::Target;
use crate::proxy::tcp_connection;
use crate::proxy::Proxy;
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::info;

pub struct TcpProxy {}

#[async_trait]
impl Proxy for TcpProxy {
    async fn accept(
        listeners: Vec<(String, TcpListener)>,
        current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
    ) -> Result<()> {
        let idx: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));

        for (name, listener) in listeners {
            // Listen to incoming traffic on separate threads
            let idx = Arc::clone(&idx);
            let current_healthy_targets = Arc::clone(&current_healthy_targets);

            tokio::spawn(async move {
                while let Ok((stream, remote_peer)) = listener.accept().await {
                    info!("Incoming request on {remote_peer}");

                    let idx = Arc::clone(&idx);
                    let tcp_targets = Arc::clone(&current_healthy_targets);
                    // Pass the TCP streams over to separate threads to avoid
                    // blocking and give each thread its copy of the configuration.
                    let target_name = name.clone();

                    tokio::spawn(async move {
                        tcp_connection(tcp_targets, target_name, idx, stream)
                            .await
                            .unwrap();
                    })
                    .await
                    .unwrap();
                }
            });
        }
        Ok(())
    }

    async fn generate_listeners(
        bind_address: String,
        targets: HashMap<String, Target>,
    ) -> Result<Vec<(String, TcpListener)>> {
        let mut tcp_bindings = vec![];

        for (name, target) in targets {
            if target.protocol_type() == Protocol::Tcp {
                let addr = format!("{}:{}", bind_address.clone(), target.listener.unwrap());
                let listener = TcpListener::bind(&addr).await?;
                info!("Binding to {} for {}", &addr, &name);
                tcp_bindings.push((name, listener));
            }
        }

        Ok(tcp_bindings)
    }
}
