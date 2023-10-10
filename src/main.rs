use std::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:9091").await?;

    loop {
        match listener.accept().await {
            Ok((mut socket, addr)) => {
                println!("New connection {}", addr);
                socket.write_all(b"OK!").await?;
            }
            Err(e) => eprintln!("Couldn't get client: {}", e),
        }
    }
}
