pub mod audio;
pub mod models;
pub mod worker;

use std::path::PathBuf;

pub fn data_directory() -> PathBuf {
    directories::ProjectDirs::from("dev", "peep", "Peep")
        .expect("macOS application support directory")
        .data_dir()
        .to_owned()
}
