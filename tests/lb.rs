use assert_cmd::prelude::*;
use gruglb;
use std::process::Command;
use std::{thread, time::Duration};

mod common;

#[test]
fn tcp_register_healthy_targets() {
    thread::spawn(move || {
        let mut cmd = Command::cargo_bin("fake_backend").unwrap();
        // No protocol defaults to TCP
        cmd.args(["--id", "fake-tcp-1", "--port", "8095"]).unwrap();
    });
    thread::spawn(move || {
        let mut cmd = Command::cargo_bin("fake_backend").unwrap();
        cmd.args(["--id", "fake-tcp-2", "--port", "8096"]).unwrap();
    });

    let test_config = common::single_tcp_target_config();

    let (send, recv) = common::get_send_recv();
    let lb = gruglb::lb::new(test_config);
    let _ = lb.run(send, recv);

    thread::sleep(Duration::from_secs(10));

    let healthy_backends = lb
        .current_healthy_targets
        .read()
        .unwrap()
        .get("tcpServersA")
        .unwrap()
        .to_owned();

    assert_eq!(healthy_backends.len(), 2);
}

#[test]
fn http_register_healthy_targets() {
    thread::spawn(move || {
        let mut cmd = Command::cargo_bin("fake_backend").unwrap();
        cmd.args([
            "--protocol",
            "http",
            "--id",
            "fake-http-1",
            "--port",
            "8097",
        ])
        .unwrap();
    });
    thread::spawn(move || {
        let mut cmd = Command::cargo_bin("fake_backend").unwrap();
        cmd.args([
            "--protocol",
            "http",
            "--id",
            "fake-http-2",
            "--port",
            "8098",
        ])
        .unwrap();
    });

    let test_config = common::single_http_target_config();

    let (send, recv) = common::get_send_recv();
    let lb = gruglb::lb::new(test_config);
    let _ = lb.run(send, recv);

    thread::sleep(Duration::from_secs(10));

    let healthy_backends = lb
        .current_healthy_targets
        .read()
        .unwrap()
        .get("webServersA")
        .unwrap()
        .to_owned();

    assert_eq!(healthy_backends.len(), 2);
}
