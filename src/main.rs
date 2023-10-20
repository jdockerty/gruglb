use anyhow::Result;
use gruglb::init;
use gruglb::lb::{RecvTargets, SendTargets};
use std::sync::mpsc::sync_channel;
use std::thread;

fn main() -> Result<()> {
    let lb = init()?;
    let channel_size = lb.conf.target_names().map_or(2, |targets| targets.len());
    let (send, recv): (SendTargets, RecvTargets) = sync_channel(channel_size);

    lb.run(send, recv)?;

    // Sleep main thread so spawned threads can run
    thread::park();
    Ok(())
}
