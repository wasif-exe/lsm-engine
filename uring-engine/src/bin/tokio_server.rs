//! Standard Production-Grade Multi-Threaded Tokio Echo Server
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    println!("=== Tokio Baseline Server running on 127.0.0.1:8080 ===");

    loop {
        let (mut socket, _) = listener.accept().await?;
        let _ = socket.set_nodelay(true);

        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                match socket.read(&mut buf).await {
                    Ok(0) => return,
                    Ok(n) => {
                        if socket.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        });
    }
}