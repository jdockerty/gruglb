use clap::Parser;
use http::{response, version, HeaderValue};
use std::{io::Write, net::TcpListener};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Port to bind the application to
    #[arg(long, default_value = "8090", env = "FAKE_BACKEND_PORT")]
    port: u16,

    /// Protocol to listen for with the server, should be one of 'tcp' or 'http'
    #[arg(long, default_value = "tcp", env = "FAKE_BACKEND_PROTOCOL")]
    protocol: String,

    /// ID of the server, used for knowing which server you are receiving responses from.
    #[arg(long, env = "FAKE_BACKEND_ID")]
    id: String,
}

pub fn run() {
    let args = Cli::parse();
    match args.protocol.to_lowercase().as_str() {
        "http" => {
            let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).unwrap();

            println!("[{}] Listening on {}", args.id, addr.local_addr().unwrap());

            while let Ok((mut stream, addr)) = addr.accept() {
                println!("Incoming from {}", addr);
                let msg = &format!("Hello from {}", args.id);
                let resp = response::Response::builder()
                    .version(version::Version::HTTP_11)
                    .header("Content-Length", msg.len())
                    .header("Content-Type", "text/plain")
                    .body(msg)
                    .unwrap();


                stream.write_all(resp.body().as_bytes()).unwrap();
            }
        }
        _ => {
            let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).unwrap();

            println!("[{}] Listening on {}", args.id, addr.local_addr().unwrap());

            while let Ok((mut stream, addr)) = addr.accept() {
                println!("Incoming from {}", addr);
                let buf = format!("Hello from {}", args.id);
                stream.write_all(buf.as_bytes()).unwrap();
            }
        }
    }
}
