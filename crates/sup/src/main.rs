#![allow(dead_code)]
mod sup;

use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/services".into())
        .into();

    let mut supervisor = sup::Supervisor::from_dir(&dir)?;
    supervisor.start_all().await;

    Ok(())
}
