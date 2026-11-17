use anyhow::{Context, Result, bail};
use peep_core::Event;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Manifest {
    repository: String,
    revision: String,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    path: String,
    bytes: u64,
    sha256: String,
}
pub struct Model {
    manifest: &'static str,
    directory: &'static str,
}

pub const COREML: Model = Model {
    manifest: include_str!("../assets/parakeet-coreml.json"),
    directory: "parakeet-v3-54f3f976",
};
pub const ONNX: Model = Model {
    manifest: include_str!("../assets/parakeet-onnx.json"),
    directory: "parakeet-v3-onnx-ed5d7456",
};

impl Model {
    fn manifest(&self) -> Manifest {
        serde_json::from_str(self.manifest).expect("bundled manifest")
    }
    pub fn directory(&self) -> PathBuf {
        crate::data_directory().join("models").join(self.directory)
    }
    pub fn download_bytes(&self) -> u64 {
        self.manifest().assets.iter().map(|x| x.bytes).sum()
    }
    pub fn verify(&self, root: &Path, cancel: &CancellationToken) -> Result<()> {
        for asset in self.manifest().assets {
            if cancel.is_cancelled() {
                bail!("Canceled");
            }
            let path = root.join(&asset.path);
            let mut file = std::fs::File::open(&path)
                .with_context(|| format!("Missing model file: {}", asset.path))?;
            if file.metadata()?.len() != asset.bytes {
                bail!("Model file has the wrong size: {}", asset.path);
            }
            let mut hash = Sha256::new();
            let mut bytes = [0u8; 64 * 1024];
            loop {
                if cancel.is_cancelled() {
                    bail!("Canceled");
                }
                let count = file.read(&mut bytes)?;
                if count == 0 {
                    break;
                }
                hash.update(&bytes[..count]);
            }
            if format!("{:x}", hash.finalize()) != asset.sha256 {
                bail!("Model checksum failed: {}", asset.path);
            }
        }
        Ok(())
    }
    pub async fn download(
        &self,
        id: u64,
        events: &async_channel::Sender<Event>,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        let destination = self.directory();
        if destination.exists() && self.verify(&destination, cancel).is_ok() {
            return Ok(destination);
        }
        let parent = destination.parent().expect("model parent");
        std::fs::create_dir_all(parent)?;
        let staging = tempfile::Builder::new()
            .prefix(".download-")
            .tempdir_in(parent)?;
        let manifest = self.manifest();
        let total: u64 = manifest.assets.iter().map(|x| x.bytes).sum();
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(30))
            .build()?;
        let mut completed = 0u64;
        for asset in &manifest.assets {
            let url = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                manifest.repository, manifest.revision, asset.path
            );
            let response = client.get(url).send();
            let mut response = tokio::select! { _ = cancel.cancelled() => bail!("Canceled"), r = response => r?.error_for_status()? };
            let output = staging.path().join(&asset.path);
            std::fs::create_dir_all(output.parent().expect("asset parent"))?;
            let mut file = std::fs::File::create(output)?;
            let mut hash = Sha256::new();
            let mut received = 0u64;
            let mut reported = completed;
            loop {
                let chunk = tokio::select! { _ = cancel.cancelled() => bail!("Canceled"), r = response.chunk() => r? };
                let Some(chunk) = chunk else {
                    break;
                };
                received += chunk.len() as u64;
                if received > asset.bytes {
                    bail!("Model server returned an oversized file.");
                }
                file.write_all(&chunk)?;
                hash.update(&chunk);
                let current = completed + received;
                if current - reported >= 1_048_576 || received == asset.bytes {
                    let _ = events.try_send(Event {
                        kind: "progress".into(),
                        id,
                        progress: current as f64 / total as f64,
                        ..Default::default()
                    });
                    reported = current;
                }
            }
            if received != asset.bytes || format!("{:x}", hash.finalize()) != asset.sha256 {
                bail!("Downloaded model failed verification: {}", asset.path);
            }
            file.sync_all()?;
            completed += received;
        }
        if cancel.is_cancelled() {
            bail!("Canceled");
        }
        let backup = parent.join(format!(".previous-{}", self.directory));
        if backup.exists() {
            std::fs::remove_dir_all(&backup)?;
        }
        if destination.exists() {
            std::fs::rename(&destination, &backup)?;
        }
        if let Err(error) = std::fs::rename(staging.path(), &destination) {
            if backup.exists() {
                let _ = std::fs::rename(&backup, &destination);
            }
            return Err(error.into());
        }
        if backup.exists() {
            let _ = std::fs::remove_dir_all(backup);
        }
        Ok(destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_checks_content_size_missing_files_and_cancellation() {
        let model = Model {
            manifest: r#"{"repository":"test/model","revision":"test","assets":[{"path":"weights","bytes":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]}"#,
            directory: "test",
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("weights");
        let cancel = CancellationToken::new();
        assert!(model.verify(root.path(), &cancel).is_err());
        std::fs::write(&path, b"abc").unwrap();
        assert!(model.verify(root.path(), &cancel).is_ok());
        std::fs::write(&path, b"abd").unwrap();
        assert!(model.verify(root.path(), &cancel).is_err());
        std::fs::write(&path, b"abcd").unwrap();
        assert!(model.verify(root.path(), &cancel).is_err());
        std::fs::write(&path, b"abc").unwrap();
        cancel.cancel();
        assert!(model.verify(root.path(), &cancel).is_err());
    }

    #[test]
    fn partial_or_corrupt_model_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            COREML
                .verify(root.path(), &CancellationToken::new())
                .is_err()
        );
        let asset = COREML.manifest().assets.remove(0);
        let path = root.path().join(asset.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0; asset.bytes as usize]).unwrap();
        assert!(
            COREML
                .verify(root.path(), &CancellationToken::new())
                .is_err()
        );
    }
}
