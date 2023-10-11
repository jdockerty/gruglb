mod config;

use rand::prelude::*;
use serde_json::json;
use std::net::{TcpListener, TcpStream};
use std::thread;

fn work(name: &str, stream: TcpStream) {
    println!("{name} starting work");
    let res = json!({"status": "OK"});
    thread::sleep(std::time::Duration::from_secs(5));
    let _ = serde_json::to_writer(stream, &res);
    println!("{name} done!");
}

fn main() {
    let ports = vec!["9091", "9092"];

    for port in ports {
        let addr = format!("127.0.0.1:{}", port);
        let listener = TcpListener::bind(&addr).unwrap();
        println!("Listening on {}", addr);

        // Listen to incoming traffic on separate threads
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        // Pass the TCP streams over to separate threads to avoid
                        // blocking.
                        thread::spawn(move || {
                            let mut rng = thread_rng();
                            let n: u32 = rng.gen();
                            let name = &format!("thread-{}", n);
                            work(name, stream);
                        });
                    }
                    Err(e) => {
                        eprintln!("Unable to connect: {}", e);
                    }
                }
            }
        });
    }

    // Sleep main thread so spawned threads can run
    thread::park();
}
