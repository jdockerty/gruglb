use crate::config::Backend;
use crate::proxy::http_connection;
use crate::proxy::Proxy;
use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::info;

pub struct HttpProxy {}

#[async_trait]
impl Proxy for HttpProxy {
    async fn accept(
        listeners: Vec<(String, TcpListener)>,
        current_healthy_targets: Arc<DashMap<String, Vec<Backend>>>,
    ) -> Result<()> {
        let idx: Arc<RwLock<usize>> = Arc::new(RwLock::new(0));

        tokio::spawn(async move {
            for (name, listener) in listeners {
                while let Ok((mut stream, address)) = listener.accept().await {
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
                    let client = Arc::new(reqwest::Client::new());
                    tokio::spawn(async move {
                        info!("{method} request at {path}");
                        http_connection(
                            client,
                            current_healthy_targets,
                            name,
                            idx,
                            method.to_string(),
                            path.to_string(),
                            stream,
                        )
                        .await
                        .unwrap();
                    });
                }
            }
        });
        Ok(())
    }
}
