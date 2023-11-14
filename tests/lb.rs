use assert_cmd::prelude::*;
use gruglb;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::{thread, time::Duration};

mod common;

#[test]
fn register_healthy_targets() {
    let pids = Arc::new(Mutex::new(vec![]));

    for n in 0..=3 {
        let pids = pids.clone();
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
        });
    }

    let test_config = common::test_targets_config();

    let (send, recv) = common::get_send_recv();
    let lb = gruglb::lb::new(test_config);
    let _ = lb.run(send, recv);

    // Wait for some set duration so that the health checks are conducted.
    thread::sleep(Duration::from_secs(10));

    let tcp_healthy_backends = lb
        .current_healthy_targets
        .read()
        .unwrap()
        .get("tcpServersA")
        .unwrap()
        .to_owned();

    let http_healthy_backends = lb
        .current_healthy_targets
        .read()
        .unwrap()
        .get("webServersA")
        .unwrap()
        .to_owned();

    // Stored our healthy targets, they're no longer needed so we can kill the
    for pid in pids.lock().unwrap().iter_mut() {
        match pid.kill() {
            Ok(_) => {}
            Err(e) => {
                let id = pid.id();
                panic!("Unable to kill process ({id}), you may have to do so manually: {e}");
            }
        }
    }

    assert_eq!(http_healthy_backends.len(), 2);
    assert_eq!(tcp_healthy_backends.len(), 2);
}
