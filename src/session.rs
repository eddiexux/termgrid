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
}

impl Session {
    pub fn session_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("termgrid")
            .join("sessions.json")
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
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
                TileSession { cwd: PathBuf::from("/tmp") },
                TileSession { cwd: PathBuf::from("/home/user") },
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
}
