#[path = "../recovery_reader.rs"]
mod recovery_reader;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    recovery_reader::run().await
}
