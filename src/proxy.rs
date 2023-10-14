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
use tracing::{debug, error, field::debug, info};

// TODO: Use this for HTTP targets!
/// Run health checks against configured HTTP targets.
//pub fn health_check(conf: Box<Config>) {
//    let duration = Duration::from_secs(1);
//
//    if let Some(targets) = &conf.targets {
//        let mut healthy_per_target: Arc<Mutex<HashMap<String, Vec<Backend>>>> =
//            Arc::new(Mutex::new(HashMap::new()));
//
//        info!("Starting health checks");
//        loop {
//            thread::sleep(duration);
//            for target in targets.values() {
//                if let Some(backends) = &target.backends {
//                    for backend in backends {
//                        let request_addr = &format!(
//                            "http://{}:{}{}",
//                            backend.host, backend.port, backend.healthcheck_path
//                        );
//                        // Simple blocking call to avoid async functions leaking everywhere.
//                        match reqwest::blocking::get(request_addr) {
//                            Ok(response) => {
//                                if response.status().is_success()
//                                    || response.status().is_redirection()
//                                {
//                                    info!("Adding healthy backend for {}", target.name);
//
//                                    let healthy_backends = match healthy_per_target.lock() {
//                                        Ok(healthy_backends) => healthy_backends,
//                                        Err(e) => {
//                                            error!("Unable to acquire lock: {e}");
//                                            e.into_inner()
//                                        }
//                                    };
//
//                                    add_healthy_backend(
//                                        target.name.clone(),
//                                        backend.clone(),
//                                        healthy_backends.clone(),
//                                    );
//                                }
//
//                                let status_code = response.status();
//                                info!("{} has unhealthy backend {} ({status_code}), removing from pool", target.name, request_addr);
//                            }
//                            Err(err) => error!("Unable to perform health check: {err}"),
//                        }
//                    }
//                } else {
//                    info!("No backends to health check for {}", target.name);
//                }
//            }
//        }
//    } else {
//        info!("No targets configured, unable to health check.");
//    }
//}

pub fn tcp_health(conf: Box<Config>) {
    let duration = Duration::from_secs(15);

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
