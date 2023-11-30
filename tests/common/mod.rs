use gruglb::config;
use gruglb::lb::{RecvTargets, SendTargets};
use std::fs::File;
use std::process::Child;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::channel;

#[derive(Clone, Debug)]
/// Helper struct for test cleanup.
pub struct Helper {
    pub pids: Arc<Mutex<Vec<Child>>>,
}

impl Drop for Helper {
    fn drop(&mut self) {
        for pid in self.pids.lock().unwrap().iter_mut() {
            match pid.kill() {
                Ok(_) => println!("Killed {}", pid.id()),
                Err(e) => {
                    let id = pid.id();
                    panic!("Unable to kill process ({id}), you may have to do so manually: {e}");
                }
            }
        }
    }
}

pub fn test_targets_config() -> config::Config {
    let fake_conf =
        File::open("tests/fixtures/example-config.yaml").expect("unable to open example config");

    config::new(fake_conf).unwrap()
}

pub fn test_http_config() -> config::Config {
    let fake_conf = File::open("tests/fixtures/http.yaml").expect("unable to open http config");

    config::new(fake_conf).unwrap()
}

pub fn get_send_recv() -> (SendTargets, RecvTargets) {
    let (send, recv): (SendTargets, RecvTargets) = channel(1);
    (send, recv)
}
