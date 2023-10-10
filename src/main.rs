use std::io::prelude::*;
use std::net::TcpListener;
use std::{io, thread};

fn main() -> io::Result<()> {
    let bind_addrs = vec!["9091", "9092"];

    let mut listeners = Vec::new();

    for b in bind_addrs {
        let addr = &format!("127.0.0.1:{}", b);
        let listener = TcpListener::bind(addr).unwrap();
        println!("Listening on {}", addr);
        listeners.push(listener);
    }

    for l in listeners {
        thread::spawn(move || {
            for stream in l.incoming() {
                let stream = stream.unwrap();
                println!("{:?}", stream.local_addr());
            }
        });
    }

    thread::park();
    Ok(())
}
