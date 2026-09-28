//! Persistent launcher state (`<data_dir>/state.json`): pinned apps, usage
//! statistics, window geometry and ports assigned to localhost-mode apps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub last_opened: u64,
    pub launches: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherState {
    pub pinned: Vec<String>,
    pub usage: HashMap<String, Usage>,
    /// Keyed by window label (`launcher`, `app-<id>`).
    pub windows: HashMap<String, WindowGeometry>,
    pub ports: HashMap<String, u16>,
}

pub struct StateStore {
    path: PathBuf,
    pub data: LauncherState,
}

impl StateStore {
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| match serde_json::from_str(&t) {
                Ok(s) => Some(s),
                Err(e) => {
                    log::warn!("ignoring corrupt state file {}: {e}", path.display());
                    None
                }
            })
            .unwrap_or_default();
        Self { path, data }
    }

    /// Atomic write (temp file + rename) so a crash never leaves a truncated file.
    pub fn save(&self) {
        if let Err(e) = write_atomic(
            &self.path,
            &serde_json::to_vec_pretty(&self.data).unwrap_or_default(),
        ) {
            log::warn!("failed to save state: {e}");
        }
    }

    pub fn record_launch(&mut self, id: &str) {
        let u = self.data.usage.entry(id.to_string()).or_default();
        u.last_opened = now_millis();
        u.launches += 1;
        self.save();
    }

    pub fn toggle_pin(&mut self, id: &str) -> bool {
        let pinned = if let Some(i) = self.data.pinned.iter().position(|p| p == id) {
            self.data.pinned.remove(i);
            false
        } else {
            self.data.pinned.push(id.to_string());
            true
        };
        self.save();
        pinned
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("webdock-state-{}", std::process::id()));
        let file = dir.join("state.json");
        let mut s = StateStore::load(file.clone());
        assert!(s.toggle_pin("a"));
        s.record_launch("a");
        s.record_launch("a");
        let s2 = StateStore::load(file);
        assert_eq!(s2.data.pinned, vec!["a"]);
        assert_eq!(s2.data.usage["a"].launches, 2);
        std::fs::remove_dir_all(dir).ok();
    }
}
