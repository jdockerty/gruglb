use std::{thread, time::Duration};

use gruglb;

mod common;

const FAKE_BACKEND_PROTOCOL: &str = "FAKE_BACKEND_PROTOCOL";
const FAKE_BACKEND_ID: &str = "FAKE_BACKEND_ID";
const FAKE_BACKEND_PORT: &str = "FAKE_BACKEND_PORT";

#[test]
fn register_healthy_targets() {
    let test_config = common::get_test_config();

    let lb = gruglb::lb::new(test_config);
    let (send, recv) = common::get_send_recv();

    thread::spawn(move || {
        std::env::set_var(FAKE_BACKEND_PORT, "8090");
        std::env::set_var(FAKE_BACKEND_ID, "fake1");
        gruglb::fakebackend::run()
    });

    thread::spawn(move || {
        std::env::set_var(FAKE_BACKEND_PORT, "8093");
        std::env::set_var(FAKE_BACKEND_ID, "fake2");
        gruglb::fakebackend::run()
    });

    let _ = lb.run(send, recv);

    // The `.run()` call spawns multiple separate threads for its work, so we can
    // sleep the main test thread in order to allow it to do its thing.
    thread::sleep(Duration::from_secs(10));

    let healthy_target_count = lb
        .current_healthy_targets
        .read()
        .unwrap()
        .get("9090")
        .unwrap()
        .len();

    assert_eq!(healthy_target_count, 2);
}
