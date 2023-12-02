use assert_cmd::prelude::*;
use std::collections::HashSet;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio_util::sync::CancellationToken;

mod common;

#[tokio::test]
async fn route_to_healthy_targets() {
    let p = common::Helper {
        pids: Arc::new(Mutex::new(vec![])),
    };

    for n in 2..=3 {
        let pids = p.pids.clone();
        thread::spawn(move || {
            let mut cmd = Command::cargo_bin("fake_backend").unwrap();

            cmd.args([
                "--id",
                &format!("fake-http-{n}"),
                "--port",
                &format!("809{n}"),
                "--protocol",
                "http",
            ]);
            let process = cmd.spawn().unwrap();
            let mut pids = pids.lock().unwrap();
            pids.push(process);
        })
        .join()
        .unwrap();
    }

    let test_config = common::test_http_config();

    let (send, recv) = common::get_send_recv();
    let lb = gruglb::lb::new(test_config.clone());
    let token = CancellationToken::new();
    lb.run(send, recv, token.child_token()).await.unwrap();

    // Ensure that the health checks run over multiple cycles by waiting more
    // than the configured duration.
    let wait_duration = test_config.health_check_interval() * 3;
    tokio::time::sleep(wait_duration).await;

    // Send some requests and ensure we see the expected responses back.
    let http_client = reqwest::Client::new();
    let mut responses = HashSet::new();

    for _ in 0..=4 {
        let response = http_client
            .get("http://localhost:8080")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        responses.insert(response.text().await.unwrap());
    }

    assert!(
        responses.contains("Hello from fake-http-2"),
        "responses did not contain 'Hello from fake-http-2'. Contains: {responses:?}"
    );
    assert!(
        responses.contains("Hello from fake-http-3"),
        "responses did not contain 'Hello from fake-http-3'. Contains: {responses:?}"
    );
    // We're using a set, so we expect to only see these 2 known responses from the fake_backend
    // servers.
    assert_eq!(responses.len(), 2);
}
