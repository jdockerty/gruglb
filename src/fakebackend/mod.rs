use anyhow::Result;
use clap::Parser;
use tokio::{io::AsyncWriteExt, net::TcpListener};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Port to bind the application to
    #[arg(long, default_value = "8090", env = "FAKE_BACKEND_PORT")]
    port: u16,

    /// Protocol to listen for with the server, should be one of 'tcp' or 'http'
    #[arg(long, default_value = "tcp", env = "FAKE_BACKEND_PROTOCOL")]
    protocol: String,

    #[arg(long, short)]
    verbose: bool,

    /// ID of the server, used for knowing which server you are receiving responses from.
    #[arg(long, env = "FAKE_BACKEND_ID")]
    id: String,
}

pub async fn run() -> Result<()> {
    let args = Cli::parse();
    match args.protocol.to_lowercase().as_str() {
        "http" => {
            let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).await?;

            println!(
                "[{}] Listening for HTTP requests on {}",
                args.id,
                addr.local_addr().unwrap()
            );

            let status_line = "HTTP/1.1 200 OK";
            let msg = &format!("Hello from {}", args.id);
            let length = msg.len();

            while let Ok((mut stream, addr)) = addr.accept().await {
                if args.verbose {
                    println!("Incoming from {}", addr);
                }
                let response =
                    format!("{status_line}\r\nContent-Length: {length}\nContent-Type: text/plain\r\n\r\n{msg}");

                stream.write_all(response.as_bytes()).await?;
            }
        }
        _ => {
            let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).await?;

            println!(
                "[{}] Listening for TCP on {}",
                args.id,
                addr.local_addr().unwrap()
            );

            let msg = format!("Hello from {}", args.id);
            while let Ok((mut stream, addr)) = addr.accept().await {
                if args.verbose {
                    println!("Incoming from {}", addr);
                }
                stream.write_all(msg.as_bytes()).await?;
            }
        }
    }
    Ok(())
}
