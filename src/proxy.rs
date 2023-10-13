use crate::config::Config;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::{Arc, Mutex},
};
use tracing::{debug, info};

// Proxy a TCP connection to a range of configured backend servers.
pub fn tcp_connection(conf: Box<Config>, routing_idx: Arc<Mutex<usize>>, mut stream: TcpStream) {
    let request_port = stream.local_addr().unwrap().port();
    info!("Incoming request on {}", &request_port);

    // We can unwrap here because we have already checked for the existence of targets.
    let targets = conf.targets.unwrap();
    let target_count = targets.len();

    if let Some(target) = targets.get(&request_port) {
        let mut backends = target.backends.unwrap();
        debug!("Retrieved backend {:?}", &backends);

        let mut index = match routing_idx.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("Unable to acquire lock: {poisoned}");
                poisoned.into_inner()
            }
        };

        if *index >= target_count {
            *index = 0;
        }

        let backend_addr = format!("{}:{}", backend.host, backend.port);
        debug!("Attempting to connect to {}", &backend_addr);

        match TcpStream::connect(backend_addr) {
            Ok(mut response) => {
                let mut buffer = Vec::new();
                response.read_to_end(&mut buffer).unwrap();
                stream.write_all(&buffer).unwrap();
                stream.shutdown(Shutdown::Both).unwrap();
                debug!("TCP stream closed");
            }
            Err(e) => eprintln!("{e}"),
        };
    } else {
        info!("No backend configured for {}", &request_port);
    };
}
