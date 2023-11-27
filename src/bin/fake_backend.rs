use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    gruglb::fakebackend::run().await
}
