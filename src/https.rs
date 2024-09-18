use crate::config::Protocol;
use crate::proxy::Connection;
use crate::proxy::Proxy;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Response;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tracing::debug;
use tracing::error;
use tracing::info;

/// `HttpsProxy` is used as a concrete implementation of the `Proxy` trait for HTTP
/// connection proxying to configured targets.
#[derive(Debug, Clone, Copy)]
pub struct HttpsProxy {}

impl Default for HttpsProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpsProxy {
    /// Return a new instance of `HttpsProxy`.
    ///
    /// `HttpsProxy` has a static lifetime as it exists the entire duration of the
    /// application's active lifecycle.
    pub fn new() -> HttpsProxy {
        Self {}
    }

    /// Helper for creating the relevant HTTP response to write into a `TcpStream`.
    async fn construct_response(&self, response: Response) -> Result<String> {
        let http_version = response.version();
        let status = response.status();
        let headers = response.headers();
        let mut header = String::default();

        // Construct headers that were in the response from the backend.
        // The `\r\n` at the beginning is important to separate header key-value
        // items from each other.
        for header_key in headers.keys() {
            if let Some(value) = headers.get(header_key) {
                header.push_str(&format!("\r\n{}: {}", header_key, value.to_str().unwrap()));
            }
        }
        let response_body = response.text().await?;
        let status_line = format!("{:?} {}", http_version, status);
        let response = format!("{status_line}{header}\r\n\r\n{response_body}");
        Ok(response)
    }
}

#[async_trait]
impl Proxy for HttpsProxy {
    fn protocol_type(&self) -> Protocol {
        Protocol::Https
    }

    /// Handles the proxying of HTTP connections to configured targets.
    async fn proxy(&self, connection: Connection, routing_idx: Arc<AtomicUsize>) -> Result<()> {
        if let Some(backends) = connection.targets.get(&connection.target_name) {
            let backend_count = backends.len();
            if backend_count == 0 {
                info!(
                    "[{}] No routable backends for {}, nothing to do",
                    self.protocol_type(),
                    &connection.target_name
                );
                return Ok(());
            }

            // TODO: All of this is shared with HTTP outside of these 2 lines.
            // Can we do something nice here to handle that?
            let tls_acceptor = connection.tls.clone().unwrap().clone();

            match tls_acceptor.accept(connection.stream).await {
                Ok(mut tls_stream) => {
                    debug!("Backends configured {:?}", &backends);
                    let buf = BufReader::new(&mut tls_stream);

                    let mut lines = buf.lines();
                    let mut http_request: Vec<String> = vec![];

                    while let Some(line) = lines.next_line().await.unwrap() {
                        if line.is_empty() {
                            break;
                        }
                        http_request.push(line);
                    }

                    let info = http_request[0].clone();
                    let http_info = info
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>();

                    let method = http_info[0].clone();
                    let request_path = http_info[1].clone();

                    // Reset index when out of bounds to route back to the first server.
                    if routing_idx.load(Ordering::Acquire) >= backend_count {
                        routing_idx.store(0, Ordering::Relaxed);
                    }

                    let backend_idx = routing_idx.load(Ordering::Relaxed);
                    let https_backend = format!(
                        "https://{}:{}{}",
                        backends[backend_idx].host, backends[backend_idx].port, request_path
                    );

                    debug!(
                        "[{}] {backend_count} backends configured for {}, current index {}",
                        self.protocol_type(),
                        &connection.target_name,
                        routing_idx.load(Ordering::Relaxed),
                    );

                    // Increment a shared index after we've constructed our current connection
                    // address.
                    routing_idx.fetch_add(1, Ordering::Relaxed);

                    info!(
                        "[{}] Attempting to connect to {}",
                        self.protocol_type(),
                        &https_backend
                    );

                    match method.as_str() {
                        "GET" => {
                            let backend_response = connection
                                .client
                                .as_ref()
                                .unwrap()
                                .get(&https_backend)
                                .send()
                                .await
                                .with_context(|| {
                                    format!("unable to send response to {https_backend}")
                                })?;
                            let response = self.construct_response(backend_response).await?;

                            tls_stream.write_all(response.as_bytes()).await?;
                        }
                        "POST" => {
                            let backend_response = connection
                                .client
                                .as_ref()
                                .unwrap()
                                .post(&https_backend)
                                .send()
                                .await
                                .with_context(|| {
                                    format!("unable to send response to {https_backend}")
                                })?;
                            let response = self.construct_response(backend_response).await?;

                            tls_stream.write_all(response.as_bytes()).await?;
                        }
                        _ => {
                            error!("Unsupported: {method}")
                        }
                    }
                    info!(
                        "[{}] response sent to {}",
                        self.protocol_type(),
                        &https_backend
                    );
                }
                Err(e) => error!("unable to accept TLS stream: {e}"),
            }
        } else {
            info!("[{}] No backend configured", self.protocol_type());
        };

        Ok(())
    }
}
