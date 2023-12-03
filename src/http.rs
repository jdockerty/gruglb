use crate::config::Backend;
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
pub struct HttpProxy {}

impl HttpProxy {
    /// Helper for creating the relevant HTTP response to write into a `TcpStream`.
    async fn construct_response(response: Response) -> Result<String> {
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
    async fn accept(
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
                    let buf = BufReader::new(&mut stream);
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
                    let path = http_info[1].clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        debug!("{method} request at {path}");
                        let connection = Connection {
                            client: Some(client),
                            targets: current_healthy_targets,
                            target_name: name,
                            method: Some(method),
                            request_path: Some(path),
                            stream,
                        };
                        HttpProxy::proxy(connection, idx).await.unwrap();
                    });
                }
            });
        }
        Ok(())
    }

    async fn proxy(mut connection: Connection, routing_idx: Arc<RwLock<usize>>) -> Result<()> {
        if let Some(backends) = connection.targets.get(&connection.target_name) {
            let backend_count = backends.len();
            if backend_count == 0 {
                info!(
                    "[HTTP] No routable backends for {}, nothing to do",
                    &connection.target_name
                );
                return Ok(());
            }
            debug!("Backends configured {:?}", &backends);

            // Limit the scope of the index write lock.
            let http_backend: String;
            {
                let mut idx = routing_idx.write().await;

                debug!(
                    "[HTTP] {backend_count} backends configured for {}, current index {idx}",
                    &connection.target_name
                );

                // Reset index when out of bounds to route back to the first server.
                if *idx >= backend_count {
                    *idx = 0;
                }

                http_backend = format!(
                    "http://{}:{}{}",
                    backends[*idx].host,
                    backends[*idx].port,
                    connection.request_path.unwrap()
                );

                // Increment a shared index after we've constructed our current connection
                // address.
                *idx += 1;
            }

            info!("[HTTP] Attempting to connect to {}", &http_backend);

            let method = connection.method.unwrap();
            match method.as_str() {
                "GET" => {
                    let backend_response = connection
                        .client
                        .unwrap()
                        .get(&http_backend)
                        .send()
                        .await
                        .with_context(|| format!("unable to send response to {http_backend}"))?;
                    let response = HttpProxy::construct_response(backend_response).await?;

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
                    let response = HttpProxy::construct_response(backend_response).await?;

                    connection.stream.write_all(response.as_bytes()).await?;
                }
                _ => {
                    error!("Unsupported: {method}")
                }
            }
            info!("[HTTP] response sent to {}", &http_backend);
        } else {
            info!("[HTTP] No backend configured");
        };

        Ok(())
    }
}
