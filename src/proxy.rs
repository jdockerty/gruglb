use crate::config::{Backend, Config};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
    vec,
};
use tracing::{debug, error, info};

// HTTP health checks for L7 targets.
// TODO: refactor common functionality between TCP/HTTP targets.
pub fn http_health(conf: Box<Config>) {
    let duration = Duration::from_secs(2);
    let health_client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();

    if let Some(targets) = &conf.targets {
        let current_healthy_targets: Arc<Mutex<HashMap<String, Vec<Backend>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        info!("Starting HTTP health checks");
        loop {
            debug!("Healthy HTTP {:?}", current_healthy_targets);
            for target in targets.values() {
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let health_uri = &format!(
                            "http://{}:{}{}",
                            backend.host, backend.port, backend.healthcheck_path
                        );
                        let mut healthy_targets = match current_healthy_targets.lock() {
                            Ok(healthy_targets) => healthy_targets,
                            Err(e) => {
                                error!("Unable to acquire lock: {e}");
                                e.into_inner()
                            }
                        };
                        // Simple blocking call to avoid async functions leaking everywhere.
                        match health_client.get(health_uri).send() {
                            Ok(response) => {
                                if response.status().is_success()
                                    || response.status().is_redirection()
                                {
                                    info!("{health_uri} is healthy backend for {}", target.name);
                                    add_backend(
                                        target.name.clone(),
                                        backend.clone(),
                                        &mut healthy_targets,
                                    );
                                }

                                if response.status().is_client_error()
                                    || response.status().is_server_error()
                                {
                                    debug!(
                                        "{} is unhealthy for {health_uri} ({}), attempting to remove",
                                        target.name,
                                        response.status()
                                    );
                                    remove_backend(
                                        target.name.clone(),
                                        backend.clone(),
                                        &mut healthy_targets,
                                    );
                                }
                            }
                            Err(err) => {
                                error!(
                                    "Unable to health check {health_uri}, attempting to remove {}: {err}",
                                    target.name
                                );
                                remove_backend(
                                    target.name.clone(),
                                    backend.clone(),
                                    &mut healthy_targets,
                                );
                            }
                        }
                    }
                } else {
                    info!("No backends to health check for {}", target.name);
                }
            }
            thread::sleep(duration);
        }
    } else {
        info!("No targets configured, unable to health check.");
    }
}

pub fn tcp_health(conf: Box<Config>) {
    let duration = Duration::from_secs(10);

    if let Some(targets) = &conf.targets {
        let healthy_tcp_targets: Arc<Mutex<HashMap<String, Vec<Backend>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        info!("Starting TCP health checks");
        loop {
            debug!("Healthy TCP: {:?}", healthy_tcp_targets);
            for target in targets.values() {
                if let Some(backends) = &target.backends {
                    for backend in backends {
                        let request_addr = &format!("{}:{}", backend.host, backend.port);
                        let mut healthy_backends = match healthy_tcp_targets.lock() {
                            Ok(healthy_backends) => healthy_backends,
                            Err(e) => {
                                error!("Unable to acquire lock: {e}");
                                e.into_inner()
                            }
                        };
                        if let Ok(_) = TcpStream::connect(request_addr) {
                            info!("{request_addr} is healthy backend for {}", target.name);
                            add_backend(
                                target.name.clone(),
                                backend.clone(),
                                &mut healthy_backends,
                            );
                        } else {
                            debug!(
                                "{request_addr} is unhealthy for {}, attempting to remove",
                                target.name
                            );
                            remove_backend(
                                target.name.clone(),
                                backend.clone(),
                                &mut healthy_backends,
                            )
                        }
                    }
                } else {
                    info!("No backends to health check for {}", target.name);
                }
            }
            thread::sleep(duration);
        }
    } else {
        info!("No targets configured, unable to health check.");
    }
}

// Proxy a TCP connection to a range of configured backend servers.
pub fn tcp_connection(conf: Box<Config>, routing_idx: Arc<Mutex<usize>>, mut stream: TcpStream) {
    let request_port = stream.local_addr().unwrap().port();
    info!("Incoming request on {}", &request_port);

    // We can unwrap here because we have already checked for the existence of targets.
    let targets = conf.targets.unwrap();

    if let Some(target) = targets.get(&request_port) {
        let backends = target.backends.clone().unwrap();
        debug!("Backends configured {:?}", &backends);
        let backend_count = backends.len();

        let mut idx = match routing_idx.lock() {
            Ok(idx) => idx,
            Err(poisoned) => poisoned.into_inner(),
        };

        debug!(
            "{backend_count} backends configured for {}, current index {idx}",
            &request_port
        );

        // Reset index when out of bounds to route back to the first server.
        if *idx >= backend_count {
            *idx = 0;
        }

        let backend_addr = format!("{}:{}", backends[*idx].host, backends[*idx].port);
        *idx += 1;

        info!("Attempting to connect to {}", &backend_addr);
        match TcpStream::connect(backend_addr) {
            Ok(mut response) => {
                let mut buffer = Vec::new();
                response.read_to_end(&mut buffer).unwrap();
                stream.write_all(&buffer).unwrap();
                stream.shutdown(Shutdown::Both).unwrap();
                debug!("TCP stream closed");
            }
            Err(e) => error!("{e}"),
        };
    } else {
        info!("No backend configured for {}", &request_port);
    };
}

fn remove_backend(
    target_name: String,
    current_backend: Backend,
    healthy_backends: &mut HashMap<String, Vec<Backend>>,
) {
    if let Some(backends) = healthy_backends.get(&target_name) {
        if !backends.contains(&current_backend) {
            debug!("Already removed, nothing to do");
            return;
        }

        // Construct a new backend vector, replacing our hashmap value with it.
        // This is simpler than remove an element in the vector and having all items
        // be shifted.
        let backends = backends.to_owned();
        let mut new_backends = Vec::new();
        for backend in &backends {
            if current_backend == *backend {
                continue;
            }
            new_backends.push(backend.clone());
        }
        healthy_backends.insert(target_name.clone(), new_backends);
        debug!("Unhealthy backend removed.");
    }
}

fn add_backend(
    target_name: String,
    current_backend: Backend,
    healthy_backends: &mut HashMap<String, Vec<Backend>>,
) {
    if let Some(backends) = healthy_backends.get(&target_name) {
        if !backends.contains(&current_backend) {
            let mut backends = backends.to_owned();
            backends.push(current_backend.clone());
            healthy_backends.insert(target_name, backends).unwrap();
            debug!("New healthy backend added");
        } else {
            debug!("Healthy backend already known, nothing to do");
        }
    } else {
        let initial_backend = vec![current_backend];
        healthy_backends.insert(target_name.clone(), initial_backend.clone());
        debug!(
            "Inserted initial backend for {}: {:?}",
            target_name.clone(),
            initial_backend.clone()
        );
    };
}
