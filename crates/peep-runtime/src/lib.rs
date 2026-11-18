pub mod audio;
pub mod models;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn data_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("PEEP_DATA_DIR") {
        return PathBuf::from(path);
    }
    directories::ProjectDirs::from("dev", "peep", "Peep")
        .expect("application data directory")
        .data_dir()
        .to_owned()
}

pub fn lock_instance() -> Result<Option<std::fs::File>> {
    lock_instance_in(&data_directory())
}

fn lock_instance_in(directory: &Path) -> Result<Option<std::fs::File>> {
    std::fs::create_dir_all(directory)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("instance.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub shortcut: String,
    pub shortcut_enabled: bool,
    pub indicator: bool,
    pub dictation_language: Option<String>,
    pub model_path: Option<PathBuf>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            shortcut: "Control+Shift+Space".into(),
            shortcut_enabled: true,
            indicator: true,
            dictation_language: None,
            model_path: None,
        }
    }
}
impl Preferences {
    pub fn load() -> Result<Self> {
        let path = data_directory().join("settings.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    pub fn save(&self) -> Result<()> {
        use std::io::Write;
        let root = data_directory();
        std::fs::create_dir_all(&root)?;
        let mut file = tempfile::NamedTempFile::new_in(&root)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(root.join("settings.json"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_lock_is_independent_of_launch_working_directory() {
        let directory = tempfile::tempdir().unwrap();
        let guard = lock_instance_in(directory.path()).unwrap().unwrap();
        assert!(lock_instance_in(directory.path()).unwrap().is_none());
        drop(guard);
        assert!(lock_instance_in(directory.path()).unwrap().is_some());
    }

    #[test]
    fn existing_preferences_survive_shared_runtime_migration() {
        let preferences: Preferences = serde_json::from_str(
            r#"{"shortcut":"Super+KeyK","shortcut_enabled":false,"indicator":false,"model_path":"/models"}"#,
        ).unwrap();
        assert_eq!(preferences.shortcut, "Super+KeyK");
        assert!(!preferences.shortcut_enabled);
        assert!(!preferences.indicator);
        assert_eq!(preferences.model_path, Some(PathBuf::from("/models")));
        let defaults: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.shortcut, Preferences::default().shortcut);
        assert_eq!(defaults.dictation_language, None);
        let selected: Preferences = serde_json::from_str(r#"{"dictation_language":"en"}"#).unwrap();
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&selected).unwrap()).unwrap();
        assert_eq!(restored.dictation_language.as_deref(), Some("en"));
    }
}
