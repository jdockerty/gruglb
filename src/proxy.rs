use crate::{
    config::{Backend, Config, Target},
    RecvTargets, SendTargets, TCP_CURRENT_HEALTHY_TARGETS,
};
use anyhow::Result;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::Duration,
    vec,
};
use tracing::{debug, error, info};

// HTTP health checks for L7 targets.
// TODO: refactor common functionality between TCP/HTTP targets.
//pub fn http_health(conf: Box<Config>) {
//    let duration = Duration::from_secs(2);
//    let health_client = reqwest::blocking::Client::builder()
//        .timeout(Duration::from_secs(1))
//        .build()
//        .unwrap();
//
//    if let Some(targets) = &conf.targets {
//        let current_healthy_targets: Arc<Mutex<HashMap<String, Vec<Backend>>>> =
//            Arc::new(Mutex::new(HashMap::new()));
//
//        info!("Starting HTTP health checks");
//        loop {
//            debug!("Healthy HTTP {:?}", current_healthy_targets);
//            for target in targets.values() {
//                if let Some(backends) = &target.backends {
//                    for backend in backends {
//                        let health_uri = &format!(
//                            "http://{}:{}{}",
//                            backend.host, backend.port, backend.healthcheck_path
//                        );
//                        let mut healthy_targets = match current_healthy_targets.lock() {
//                            Ok(healthy_targets) => healthy_targets,
//                            Err(e) => {
//                                error!("Unable to acquire lock: {e}");
//                                e.into_inner()
//                            }
//                        };
//                        // Simple blocking call to avoid async functions leaking everywhere.
//                        match health_client.get(health_uri).send() {
//                            Ok(response) => {
//                                if response.status().is_success()
//                                    || response.status().is_redirection()
//                                {
//                                    info!("{health_uri} is healthy backend for {}", target.name);
//                                }
//
//                                if response.status().is_client_error()
//                                    || response.status().is_server_error()
//                                {
//                                    debug!(
//                                        "{} is unhealthy for {health_uri} ({}), attempting to remove",
//                                        target.name,
//                                        response.status()
//                                    );
//                                }
//                            }
//                            Err(err) => {
//                                error!(
//                                    "Unable to health check {health_uri}, attempting to remove {}: {err}",
//                                    target.name
//                                );
//                            }
//                        }
//                    }
//                } else {
//                    info!("No backends to health check for {}", target.name);
//                }
//            }
//            thread::sleep(duration);
//        }
//    } else {
//        info!("No targets configured, unable to health check.");
//    }
//}

/// Run health checks against the configured TCP targets.
pub fn tcp_health(conf: Arc<Config>, sender: SendTargets) {
    let duration = Duration::from_secs(5);

    if let Some(targets) = &conf.targets {
        info!("Starting TCP health checks");
        loop {
            let mut healthy_backends: Vec<Backend> = vec![];
            let mut healthy_targets = HashMap::new();
            for (name, target) in targets {
                healthy_targets.insert(name.to_string(), healthy_backends.clone());
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!("{}:{}", backend.host, backend.port);

                        if let Ok(stream) = TcpStream::connect(request_addr) {
                            stream.shutdown(Shutdown::Both).unwrap();
                            info!("{request_addr} is healthy backend for {}", name);
                            healthy_backends.push(backend.clone());
                        } else {
                            // This is "removed from the pool" because it is not included in
                            // the vector for the next channel transmission, so traffic does not get routed
                            // to it.
                            debug!(
                                "{request_addr} is unhealthy for {}, removing from pool",
                                name
                            );
                        }
                    }
                    healthy_targets.insert(name.to_string(), healthy_backends.clone());
                } else {
                    info!("No backends to health check for {}", name);
                }
            }
            debug!("Sending targets to channel");
            sender.send(healthy_targets).unwrap();
            thread::sleep(duration);
        }
    } else {
        info!("No targets configured, unable to health check.");
    }
}

// Proxy a TCP connection to a range of configured backend servers.
pub fn tcp_connection(
    targets: Arc<RwLock<HashMap<String, Vec<Backend>>>>,
    target_name: String,
    routing_idx: Arc<Mutex<usize>>,
    mut stream: TcpStream,
) -> Result<()> {
    let request_port = stream.local_addr().unwrap().port();
    info!("Incoming request on {}", &request_port);

    if let Some(backends) = targets.read().unwrap().get(&target_name) {
        let backends = backends.to_vec();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        if backend_count == 0 {
            info!("No routable backends for {target_name}, nothing to do");
            return Ok(());
        }

        let mut idx = match routing_idx.lock() {
            Ok(idx) => idx,
            Err(poisoned) => poisoned.into_inner(),
        };

        debug!("{backend_count} backends configured for {target_name}, current index {idx}");

        // Reset index when out of bounds to route back to the first server.
        if *idx >= backend_count {
            *idx = 0;
        }

        let backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);

        // Increment a shared index after we've constructed our current connection
        // address.
        *idx += 1;

        info!("Attempting to connect to {}", &backend_addr);
        match TcpStream::connect(backend_addr) {
            Ok(mut response) => {
                let mut buffer = Vec::new();
                response.read_to_end(&mut buffer)?;
                stream.write_all(&buffer)?;
                stream.shutdown(Shutdown::Both)?;
                debug!("TCP stream closed");
            }
            Err(e) => {
                stream.shutdown(Shutdown::Both)?;
                error!("{e}")
            }
        };
    } else {
        info!("No backend configured for {}", &request_port);
    };

    Ok(())
}

fn bind_tcp_listeners(conf: Arc<Config>, targets: HashMap<String, Target>) -> Result<()> {
    let idx: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let listen_addr = &conf.address;

    for (name, target) in targets {
        // Assumes always using TCP for now.
        let addr = format!("{}:{}", listen_addr.clone(), target.listener.unwrap());
        let listener = TcpListener::bind(&addr)?;
        info!("Listening on {} for {}", &addr, &name);

        // Listen to incoming traffic on separate threads
        let idx = Arc::clone(&idx);
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let idx = Arc::clone(&idx);
                        let tcp_targets = Arc::clone(&TCP_CURRENT_HEALTHY_TARGETS);
                        // Pass the TCP streams over to separate threads to avoid
                        // blocking and give each thread its copy of the configuration.
                        let target_name = name.clone();
                        thread::spawn(move || {
                            tcp_connection(tcp_targets, target_name, idx, stream)
                        });
                    }
                    Err(e) => {
                        error!("Unable to connect: {}", e);
                    }
                }
            }
        });
    }
    Ok(())
}

pub fn run(
    conf: Arc<Config>,
    targets: HashMap<String, Target>,
    sender: SendTargets,
    receiver: RecvTargets,
) -> Result<()> {
    if let Some(target_names) = conf.target_names() {
        debug!("All loaded targets {:?}", target_names);
    }

    // Provides the health check thread with its own configuration.
    let health_check_conf = conf.clone();
    thread::spawn(move || tcp_health(health_check_conf, sender));
    let healthy_targets = Arc::clone(&TCP_CURRENT_HEALTHY_TARGETS);

    // Continually receive from the channel and update our healthy backend state.
    thread::spawn(move || loop {
        for (target, backends) in receiver.recv().unwrap() {
            healthy_targets.write().unwrap().insert(target, backends);
        }
        thread::sleep(Duration::from_secs(2));
    });

    bind_tcp_listeners(conf, targets)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tcp_health_registers_correctly() {
        todo!()
    }
}
