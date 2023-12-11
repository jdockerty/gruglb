use assert_cmd::prelude::*;
use gruglb::lb::LB;
use std::collections::HashSet;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio_util::sync::CancellationToken;

mod common;

#[tokio::test]
async fn route_tls_target() {
    let p = common::Helper {
        pids: Arc::new(Mutex::new(vec![])),
    };

    for n in 1..=2 {
        let pids = p.pids.clone();
        thread::spawn(move || {
            let mut cmd = Command::cargo_bin("fake_backend").unwrap();

            // HTTP backends, TLS which is terminated at the LB.
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

    let test_config = common::test_https_config();

    let (send, recv) = common::get_send_recv();
    let lb = LB::new(test_config.clone());
    let token = CancellationToken::new();
    lb.run(send, recv, token.child_token()).await.unwrap();

    // Ensure that the health checks run over multiple cycles by waiting more
    // than the configured duration.
    let wait_duration = test_config.health_check_interval() * 3;
    tokio::time::sleep(wait_duration).await;

    let https_client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("unable to build cert");
    let mut responses = HashSet::new();

    for _ in 0..=4 {
        let response = https_client
            .get("https://localhost:8443")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        responses.insert(response.text().await.unwrap());
    }

    assert!(
        responses.contains("Hello from fake-http-1"),
        "responses did not contain 'Hello from fake-http-1'. Contains: {responses:?}"
    );
    assert!(
        responses.contains("Hello from fake-http-2"),
        "responses did not contain 'Hello from fake-http-2'. Contains: {responses:?}"
    );
    // We're using a set, so we expect to only see these 2 known responses from the fake_backend
    // servers.
    assert_eq!(responses.len(), 2);
}
