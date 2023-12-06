use crate::config::Backend;
use crate::config::Protocol;
use crate::proxy::Connection;
use crate::proxy::Proxy;
use anyhow::{Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use reqwest::Response;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::error;
use tracing::info;

/// `HttpProxy` is used as a concrete implementation of the `Proxy` trait for HTTP
/// connection proxying to configured targets.
#[derive(Debug)]
pub struct HttpProxy {}

impl HttpProxy {
    /// Return a new instance of `HttpProxy`.
    ///
    /// `HttpProxy` has a static lifetime as it exists the entire duration of the
    /// application's active lifecycle.
    pub fn new() -> &'static HttpProxy {
        &Self {}
    }

    // TODO: Pass connection here for specific HTTP requirement for request_path and method,
    // rather than doing it inline of the proxy func.
    // async fn construct_request_path_and_method(&self, mut connection: Connection) -> (String, String) {}

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
impl Proxy for HttpProxy {
    fn protocol_type(&self) -> Protocol {
        Protocol::Http
    }

    async fn accept(
        &'static self,
        listeners: Vec<(String, TcpListener)>,
        current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let idx: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));
        for (name, listener) in listeners {
            let idx = idx.clone();
            let client = Arc::new(reqwest::Client::new());
            let current_healthy_targets = current_healthy_targets.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                while let Ok((mut stream, address)) = listener.accept().await {
                    if cancel.is_cancelled() {
                        info!("[CANCEL] Received cancel, no longer receiving any HTTP requests.");
                        stream.shutdown().await.unwrap();
                        break;
                    }
                    let name = name.clone();
                    let idx = Arc::clone(&idx);
                    let current_healthy_targets = Arc::clone(&current_healthy_targets);
                    info!("Incoming HTTP request from {address}");
                    let client = client.clone();
                    tokio::spawn(async move {
                        let connection = Connection {
                            client: Some(client),
                            targets: current_healthy_targets,
                            target_name: name,
                            stream,
                        };
                        self.proxy(connection, idx).await.unwrap();
                    });
                }
            });
        }
        Ok(())
    }

    async fn proxy(
        &'static self,
        mut connection: Connection,
        routing_idx: Arc<RwLock<usize>>,
    ) -> Result<()> {
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
            debug!("Backends configured {:?}", &backends);

            let buf = BufReader::new(&mut connection.stream);
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

            // Limit the scope of the index write lock.
            let http_backend: String;
            {
                let mut idx = routing_idx.write().await;

                debug!(
                    "[{}] {backend_count} backends configured for {}, current index {idx}",
                    self.protocol_type(),
                    &connection.target_name
                );

                // Reset index when out of bounds to route back to the first server.
                if *idx >= backend_count {
                    *idx = 0;
                }

                http_backend = format!(
                    "http://{}:{}{}",
                    backends[*idx].host, backends[*idx].port, request_path
                );

                // Increment a shared index after we've constructed our current connection
                // address.
                *idx += 1;
            }

            info!(
                "[{}] Attempting to connect to {}",
                self.protocol_type(),
                &http_backend
            );

            match method.as_str() {
                "GET" => {
                    let backend_response = connection
                        .client
                        .unwrap()
                        .get(&http_backend)
                        .send()
                        .await
                        .with_context(|| format!("unable to send response to {http_backend}"))?;
                    let response = self.construct_response(backend_response).await?;

                    connection.stream.write_all(response.as_bytes()).await?;
                }
                "POST" => {
                    let backend_response = connection
                        .client
                        .unwrap()
                        .post(&http_backend)
                        .send()
                        .await
                        .with_context(|| format!("unable to send response to {http_backend}"))?;
                    let response = self.construct_response(backend_response).await?;

                    connection.stream.write_all(response.as_bytes()).await?;
                }
                _ => {
                    error!("Unsupported: {method}")
                }
            }
            info!(
                "[{}] response sent to {}",
                self.protocol_type(),
                &http_backend
            );
        } else {
            info!("[{}] No backend configured", self.protocol_type());
        };

        Ok(())
    }
}
