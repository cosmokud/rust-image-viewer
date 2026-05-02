use directories::BaseDirs;
use std::path::PathBuf;

pub const APP_DIR_NAME: &str = "rust-image-viewer";

pub fn app_config_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.config_dir().join(APP_DIR_NAME))
}

pub fn app_local_data_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.data_local_dir().join(APP_DIR_NAME))
}
