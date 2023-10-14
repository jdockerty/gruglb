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

    if let Some(target) = targets.get(&request_port) {
        let backends = target.backends.clone().unwrap();
        let backend_count = backends.len();

        let mut idx = match routing_idx.lock() {
            Ok(idx) => idx,
            Err(poisoned) => poisoned.into_inner(),
        };

        debug!("{backend_count} for {}, current index {idx}", &request_port);
        debug!("Backends configured {:?}", &backends);

        // Reset index when out of bounds to route back to the first server.
        if *idx >= backend_count {
            *idx = 0;
        }

        let backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);
        *idx += 1;

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
