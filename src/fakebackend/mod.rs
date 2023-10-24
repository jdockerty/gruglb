use clap::Parser;
use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming as IncomingBody};
use hyper::server::conn::http1;
use hyper::service::{service_fn, Service};
use hyper::{body::Body, Request, Response};
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::pin::Pin;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

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

struct Svc {
    msg: String,
}

impl Service<Request<IncomingBody>> for Svc {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<IncomingBody>) -> Self::Future {
        fn hello(msg: String) -> Result<Response<Full<Bytes>>, hyper::Error> {
            Ok(Response::builder()
                .body(Full::new(Bytes::from(msg)))
                .unwrap())
        }
        let res = match req.uri().path() {
            _ => hello(format!("Hello from {}", self.msg)),
        };
        Box::pin(async { res })
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Cli::parse();
    match args.protocol.to_lowercase().as_str() {
        "http" => {
            let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).await?;
            println!(
                "[{}] Listening on {}",
                args.id.clone(),
                addr.local_addr().unwrap()
            );
            loop {
                let (stream, _) = addr.accept().await?;

                // Use an adapter to access something implementing `tokio::io` traits as if they implement
                // `hyper::rt` IO traits.
                let io = TokioIo::new(stream);

                let msg = format!("Hello from {}", args.id.clone());
                // Spawn a tokio task to serve multiple connections concurrently
                tokio::task::spawn(async move {
                    // Finally, we bind the incoming connection to our `hello` service
                    if let Err(err) = http1::Builder::new()
                        // `service_fn` converts our function in a `Service`
                        .serve_connection(io, Svc { msg })
                        .await
                    {
                        println!("Error serving connection: {:?}", err);
                    }
                });
            }
        }
        // Default to TCP
        _ => {
            let addr = TcpListener::bind(format!("127.0.0.1:{}", args.port)).await?;

            println!("[{}] Listening on {}", args.id, addr.local_addr().unwrap());

            loop {
                let (mut stream, addr) = addr.accept().await?;
                println!("Incoming from {}", addr);
                let buf = format!("Hello from {}", args.id);
                stream.write_all(buf.as_bytes()).await?;
            }
        }
    };
}
