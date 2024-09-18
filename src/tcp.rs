use crate::config::Protocol;
use crate::proxy::Connection;
use crate::proxy::Proxy;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::debug;
use tracing::info;

/// `TcpProxy` is used as a concrete implementation of the `Proxy` trait for TCP
/// connection proxying to configured targets.
#[derive(Debug, Clone, Copy)]
pub struct TcpProxy {}

impl Default for TcpProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpProxy {
    /// Return a new instance of `TcpProxy`.
    ///
    /// `TcpProxy` has a static lifetime as it exists the entire duration of the
    /// application's active lifecycle.
    pub fn new() -> TcpProxy {
        Self {}
    }
}

#[async_trait]
impl Proxy for TcpProxy {
    fn protocol_type(&self) -> Protocol {
        Protocol::Tcp
    }

    /// Handles the proxying of TCP connections to configured targets.
    async fn proxy(&self, mut connection: Connection, routing_idx: Arc<AtomicUsize>) -> Result<()> {
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

            let backend_addr = format!(
                "{}:{}",
                backends[routing_idx.load(Ordering::Relaxed)].host,
                backends[routing_idx.load(Ordering::Relaxed)].port
            );
            debug!(
                "[TCP] {backend_count} backends configured for {}, current index {}",
                &connection.target_name,
                routing_idx.load(Ordering::Acquire),
            );

            // Reset index when out of bounds to route back to the first server.
            if routing_idx.load(Ordering::Relaxed) >= backend_count {
                routing_idx.store(0, Ordering::Relaxed);
            }

            // Increment a shared index after we've constructed our current connection
            // address.
            routing_idx.fetch_add(1, Ordering::Relaxed);

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
