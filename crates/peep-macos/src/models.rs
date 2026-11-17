use peep_runtime::models::COREML;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

pub fn model_directory() -> PathBuf {
    COREML.directory()
}
pub fn download_bytes() -> u64 {
    COREML.download_bytes()
}
pub fn verify(root: &Path, cancel: &CancellationToken) -> anyhow::Result<()> {
    COREML.verify(root, cancel)
}
pub async fn download(
    id: u64,
    events: &async_channel::Sender<peep_core::Event>,
    cancel: &CancellationToken,
) -> anyhow::Result<PathBuf> {
    COREML.download(id, events, cancel).await
}
