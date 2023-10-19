use clap::Parser;
use std::{io::Write, net::TcpListener};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Port to bind the application to
    #[arg(long, default_value = "8090")]
    port: u16,

    /// Protocol to listen for with the server, should be one of 'tcp' or 'http'
    #[arg(long, default_value = "tcp")]
    protocol: String,

    /// ID of the server, used for knowing which server you are receiving responses from.
    #[arg(long)]
    id: String,
}

fn main() {
    let args = Cli::parse();
    let protocol = match args.protocol.to_lowercase().as_str() {
        "http" => "http".to_string(),
        _ => "tcp".to_string(),
    };

    if protocol == "http" {
        todo!();
    } else {
        let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).unwrap();

        println!("[{}] Listening on {}", args.id, addr.local_addr().unwrap());

        while let Ok((mut stream, addr)) = addr.accept() {
            println!("Incoming from {}", addr);
            let buf = format!("Hello from {}", args.id);
            stream.write_all(buf.as_bytes()).unwrap();
        }
    }
}
