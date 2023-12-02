use anyhow::Result;
use gruglb::init;
use gruglb::lb::{RecvTargets, SendTargets};
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    let lb = init()?;
    let channel_size = lb.conf.target_names().map_or(2, |targets| targets.len());
    let (send, recv): (SendTargets, RecvTargets) = channel(channel_size);

    let token = CancellationToken::new();

    lb.run(send, recv, token.child_token()).await?;

    match tokio::signal::ctrl_c().await {
        Ok(_) => {
            token.cancel();
            info!("RECEIVED CANCEL OPERATION, COMMENCING 30 GRACE PERIOD BEFORE SHUTDOWN");
            // Display the shut down message 6 times, every 5 seconds before terminating.
            for i in 1..=6 {
                info!("[{i}/6] CANCEL OPERATION RECEIVED: TERMINTATING PROCESSES");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
            info!("30 second grace period has elapsed, shutting down.");
        }
        Err(e) => error!("Error sending SIGINT: {e}"),
    }

    Ok(())
}
