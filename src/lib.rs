use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientCommand {
    Store { slot: usize },
    Fetch { slot: usize },
    Swap { slot: usize },
    Status,
    Subscribe,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClientResponse {
    Success,
    Status(Vec<SlotInfo>),
    Error { message: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SlotInfo {
    pub slot: usize,
    pub window_id: Option<u64>,
    pub title: Option<String>,
}

/// Returns the socket path for communication between ctl and daemon
pub fn get_control_socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("niri-shelf.sock")
}

/// Returns the cache path for a window's Niri screenshot preview.
pub fn get_thumbnail_path(window_id: u64) -> Result<PathBuf, String> {
    let cache_dir = match std::env::var_os("XDG_CACHE_HOME") {
        Some(path) => PathBuf::from(path),
        None => {
            PathBuf::from(std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?)
                .join(".cache")
        }
    };
    Ok(cache_dir
        .join("niri-shelf")
        .join(format!("window-{window_id}.png")))
}
