use anyhow::Result;
use gruglb::init;
use gruglb::lb::{RecvTargets, SendTargets};
use std::thread;
use tokio::sync::mpsc::channel;

#[tokio::main]
async fn main() -> Result<()> {
    let lb = init()?;
    let channel_size = lb.conf.target_names().map_or(2, |targets| targets.len());
    let (send, recv): (SendTargets, RecvTargets) = channel(channel_size);

    lb.run(send, recv).await?;

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
