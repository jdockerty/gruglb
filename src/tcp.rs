use crate::config::Backend;
use crate::config::Protocol;
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
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::info;

/// `TcpProxy` is used as a concrete implementation of the `Proxy` trait for TCP
/// connection proxying to configured targets.
#[derive(Debug)]
pub struct TcpProxy {}

impl TcpProxy {
    /// Return a new instance of `TcpProxy`.
    ///
    /// `TcpProxy` has a static lifetime as it exists the entire duration of the
    /// application's active lifecycle.
    pub fn new() -> &'static TcpProxy {
        &Self {}
    }
}

#[async_trait]
impl Proxy for TcpProxy {

    fn protocol_type(&self) -> Protocol {
        Protocol::Tcp
    }

    /// Handles the proxying of TCP connections to configured targets.
    async fn proxy(
        &'static self,
        mut connection: Connection,
        routing_idx: Arc<RwLock<usize>>,
    ) -> Result<()> {
        if let Some(backends) = connection.targets.get(&connection.target_name) {
            let backend_count = backends.len();
            if backend_count == 0 {
                info!(
                    "[TCP] No routable backends for {}, nothing to do",
                    &connection.target_name
                );
                return Ok(());
            }
            debug!("Backends configured {:?}", &backends);

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
