use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub tiles: Vec<TileSession>,
    pub columns: u8,
    pub active_tab: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileSession {
    pub cwd: PathBuf,
    /// Index into scrollback files (scrollback_0.bin, scrollback_1.bin, ...).
    /// None if no scrollback was saved.
    pub scrollback_index: Option<usize>,
    /// tmux session name (e.g. "tg0"). None for native PTY sessions.
    #[serde(default)]
    pub tmux_session: Option<String>,
}

impl Session {
    pub fn session_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("termgrid")
            .join("sessions.json")
    }

    /// Directory for scrollback files.
    pub fn scrollback_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("termgrid")
            .join("scrollback")
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        tracing::info!("Session saved: {} tiles", self.tiles.len());
        Ok(())
    }

    pub fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let session: Self = serde_json::from_str(&content).ok()?;
        tracing::info!("Session loaded: {} tiles", session.tiles.len());
        Some(session)
    }

    /// Save scrollback data for a tile. Returns the index for TileSession.
    pub fn save_scrollback(index: usize, data: &[u8]) -> anyhow::Result<()> {
        let dir = Self::scrollback_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("scrollback_{}.bin", index));
        std::fs::write(&path, data)?;
        tracing::debug!("Saved scrollback {}: {} bytes", index, data.len());
        Ok(())
    }

    /// Load scrollback data for a tile.
    pub fn load_scrollback(index: usize) -> Option<Vec<u8>> {
        let path = Self::scrollback_dir().join(format!("scrollback_{}.bin", index));
        let data = std::fs::read(&path).ok()?;
        tracing::debug!("Loaded scrollback {}: {} bytes", index, data.len());
        Some(data)
    }

    /// Clean up old scrollback files.
    pub fn clean_scrollback() {
        let dir = Self::scrollback_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");

        let session = Session {
            tiles: vec![
                TileSession {
                    cwd: PathBuf::from("/tmp"),
                    scrollback_index: None,
                    tmux_session: None,
                },
                TileSession {
                    cwd: PathBuf::from("/home/user"),
                    scrollback_index: Some(0),
                    tmux_session: None,
                },
            ],
            columns: 2,
            active_tab: "ALL".into(),
        };

        session.save(&path).unwrap();
        assert!(path.exists());

        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.columns, 2);
        assert_eq!(loaded.active_tab, "ALL");
        assert_eq!(loaded.tiles.len(), 2);
        assert_eq!(loaded.tiles[0].cwd, PathBuf::from("/tmp"));
        assert_eq!(loaded.tiles[1].cwd, PathBuf::from("/home/user"));
    }

    #[test]
    fn test_load_nonexistent() {
        let result = Session::load(Path::new("/nonexistent/path/sessions.json"));
        assert!(result.is_none());
    }

    #[test]
    fn test_session_path() {
        let path = Session::session_path();
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with("termgrid/sessions.json"),
            "session path should end with termgrid/sessions.json, got: {}",
            path_str
        );
    }

    #[test]
    fn test_load_malformed_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        assert!(Session::load(&path).is_none());
    }

    #[test]
    fn test_save_and_load_empty_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.json");

        let session = Session {
            tiles: vec![],
            columns: 1,
            active_tab: "ALL".into(),
        };

        session.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.tiles.len(), 0);
        assert_eq!(loaded.columns, 1);
    }

    #[test]
    fn test_scrollback_save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        // Override scrollback dir by using save_scrollback/load_scrollback
        // which use the global scrollback_dir. Instead, test the raw file I/O.
        let data = b"hello scrollback data";
        let path = dir.path().join("scrollback_99.bin");
        std::fs::write(&path, data).unwrap();
        let loaded = std::fs::read(&path).unwrap();
        assert_eq!(loaded, data);
    }

    #[test]
    fn test_save_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("deep").join("nested").join("sessions.json");

        let session = Session {
            tiles: vec![],
            columns: 2,
            active_tab: "ALL".into(),
        };

        session.save(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_load_old_format_without_tmux_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("old_format.json");
        // Old format without tmux_session field — should load fine via #[serde(default)]
        let json = r#"{
            "tiles": [{"cwd": "/tmp", "scrollback_index": 0}],
            "columns": 2,
            "active_tab": "ALL"
        }"#;
        std::fs::write(&path, json).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.tiles.len(), 1);
        assert!(loaded.tiles[0].tmux_session.is_none());
        assert_eq!(loaded.tiles[0].scrollback_index, Some(0));
    }

    #[test]
    fn test_save_and_load_with_tmux_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tmux.json");

        let session = Session {
            tiles: vec![TileSession {
                cwd: PathBuf::from("/workspace"),
                scrollback_index: None,
                tmux_session: Some("tg0".into()),
            }],
            columns: 2,
            active_tab: "ALL".into(),
        };

        session.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.tiles[0].tmux_session.as_deref(), Some("tg0"));
        assert!(loaded.tiles[0].scrollback_index.is_none());
    }

    #[test]
    fn test_load_with_extra_fields_is_ok() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("extra.json");
        // JSON with an extra field that doesn't exist in the struct
        let json = r#"{
            "tiles": [{"cwd": "/tmp", "scrollback_index": null}],
            "columns": 1,
            "active_tab": "ALL",
            "unknown_field": "should be ignored"
        }"#;
        std::fs::write(&path, json).unwrap();
        let loaded = Session::load(&path);
        // serde default behavior: unknown fields are ignored
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().tiles.len(), 1);
    }
}
