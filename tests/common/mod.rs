use gruglb::config;
use gruglb::lb::{RecvTargets, SendTargets};
use std::fs::File;
use tokio::sync::mpsc::channel;

pub fn test_targets_config() -> config::Config {
    let fake_conf =
        File::open("tests/fixtures/example-config.yaml").expect("unable to open example config");

    config::new(fake_conf).unwrap()
}

pub fn get_send_recv() -> (SendTargets, RecvTargets) {
    let (send, recv): (SendTargets, RecvTargets) = channel(2);
    (send, recv)
}
