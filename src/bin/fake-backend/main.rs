use clap::Parser;
use std::{io::Write, net::TcpListener};
use uuid;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Port to bind the application to
    #[arg(long, default_value = "8090")]
    port: u16,

    #[arg(long, default_value = "tcp")]
    protocol: String,
}

fn main() {
    let args = Cli::parse();
    let uuid = uuid::Uuid::new_v4();
    let protocol = match args.protocol.to_lowercase().as_str() {
        "http" => "http".to_string(),
        _ => "tcp".to_string(),
    };

    if protocol == "http" {
        todo!();
    } else {
        let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).unwrap();

        println!("[{}] Listening on {}", uuid, addr.local_addr().unwrap());

        while let Ok((mut stream, addr)) = addr.accept() {
            println!("Incoming from {}", addr);
            let buf = String::from(format!("Hello from backend {}", uuid));
            stream.write_all(buf.as_bytes()).unwrap();
        }
    }
}
