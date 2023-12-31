use assert_cmd::prelude::*;
use gruglb::lb::LB;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use tokio_util::sync::CancellationToken;

mod common;

#[tokio::test]
async fn register_healthy_targets() {
    let p = common::Helper {
        pids: Arc::new(Mutex::new(vec![])),
    };

    for n in 0..=3 {
        let pids = p.pids.clone();
        thread::spawn(move || {
            let mut cmd = Command::cargo_bin("fake_backend").unwrap();

            if n < 2 {
                cmd.args([
                    "--id",
                    &format!("fake-tcp-{n}"),
                    "--port",
                    &format!("809{n}"),
                ]);
            } else {
                cmd.args([
                    "--id",
                    &format!("fake-http-{n}"),
                    "--port",
                    &format!("809{n}"),
                    "--protocol",
                    "http",
                ]);
            }
            let process = cmd.spawn().unwrap();
            let mut pids = pids.lock().unwrap();
            pids.push(process);
        })
        .join()
        .unwrap();
    }

    let test_config = common::test_targets_config();

    let (send, recv) = common::get_send_recv();
    let lb = LB::new(test_config.clone());
    let token = CancellationToken::new();
    lb.run(send, recv, token.child_token()).await.unwrap();

    // Ensure that the health checks run over multiple cycles by waiting more
    // than the configured duration.
    let wait_duration = test_config.health_check_interval() * 10;
    tokio::time::sleep(wait_duration).await;

    let tcp_healthy_backends = lb
        .current_healthy_targets
        .get("tcpServersA")
        .unwrap()
        .to_owned();

    let http_healthy_backends = lb
        .current_healthy_targets
        .get("webServersA")
        .unwrap()
        .to_owned();

    assert_eq!(
        tcp_healthy_backends.len(),
        2,
        "TCP healthy backends was {tcp_healthy_backends:?}"
    );
    assert_eq!(
        http_healthy_backends.len(),
        2,
        "HTTP healthy backends was {http_healthy_backends:?}"
    );
}
