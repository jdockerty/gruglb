use gruglb::config;
use gruglb::lb::{RecvTargets, SendTargets};
use std::fs::File;
use std::sync::mpsc::sync_channel;

pub fn single_tcp_target_config() -> config::Config {
    let fake_conf =
        File::open("tests/fixtures/single-tcp-target.yaml").expect("unable to open example config");

    config::new(fake_conf).unwrap()
}

pub fn single_http_target_config() -> config::Config {
    let fake_conf = File::open("tests/fixtures/single-http-target.yaml")
        .expect("unable to open example config");

    config::new(fake_conf).unwrap()
}

pub fn get_send_recv() -> (SendTargets, RecvTargets) {
    let (send, recv): (SendTargets, RecvTargets) = sync_channel(2);
    (send, recv)
}
