use assert_cmd::prelude::{OutputAssertExt, *};
use gruglb;
use predicates::prelude::*;
use std::process::Command;
use std::sync::Arc;
use std::{thread, time::Duration};

mod common;

const FAKE_BACKEND_PROTOCOL: &str = "FAKE_BACKEND_PROTOCOL";
const FAKE_BACKEND_ID: &str = "FAKE_BACKEND_ID";
const FAKE_BACKEND_PORT: &str = "FAKE_BACKEND_PORT";

#[test]
fn register_healthy_targets() {
    thread::spawn(move || {
        let mut cmd = Command::cargo_bin("fake_backend").unwrap();
        cmd.args(["--id", "fake-1", "--port", "8095"]).unwrap();
    });
    thread::spawn(move || {
        let mut cmd = Command::cargo_bin("fake_backend").unwrap();
        cmd.args(["--id", "fake-2", "--port", "8096"]).unwrap();
    });

    let test_config = common::get_single_target_config();

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
