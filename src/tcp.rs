use crate::config::Backend;
use crate::proxy::Connection;
use crate::proxy::Proxy;
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tracing::debug;
use tracing::info;

/// `TcpProxy` is used as a concrete implementation of the `Proxy` trait for TCP
/// connection proxying to configured targets.
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

                    let connection = Connection {
                        targets: tcp_targets,
                        stream,
                        client: None,
                        target_name,
                        method: None,
                        request_path: None,
                    };

                    tokio::spawn(async move {
                        TcpProxy::proxy(connection, idx).await.unwrap();
                    })
                    .await
                    .unwrap();
                }
            });
        }
        Ok(())
    }

    async fn proxy(mut connection: Connection, routing_idx: Arc<RwLock<usize>>) -> Result<()> {
        if let Some(backends) = connection.targets.get(&connection.target_name) {
            let backends = backends.to_vec();
            debug!("Backends configured {:?}", &backends);
            let backend_count = backends.len();

            if backend_count == 0 {
                info!(
                    "[TCP] No routable backends for {}, nothing to do",
                    &connection.target_name
                );
                return Ok(());
            }

            // Limit the scope of the index write lock.
            let backend_addr: String;
            {
                let mut idx = routing_idx.write().await;
                debug!(
                    "[TCP] {backend_count} backends configured for {}, current index {idx}",
                    &connection.target_name
                );

                // Reset index when out of bounds to route back to the first server.
                if *idx >= backend_count {
                    *idx = 0;
                }

                backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);

                // Increment a shared index after we've constructed our current connection
                // address.
                *idx += 1;
            }

            info!("[TCP] Attempting to connect to {}", &backend_addr);

            let mut response = TcpStream::connect(&backend_addr).await?;
            let mut buffer = Vec::new();
            response.read_to_end(&mut buffer).await?;
            connection.stream.write_all(&buffer).await?;
            debug!("TCP stream closed");
        } else {
            info!("[TCP] No backend configured");
        };

        Ok(())
    }
}