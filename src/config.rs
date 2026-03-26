use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub layout: LayoutConfig,
    pub scan: ScanConfig,
    pub terminal: TerminalConfig,
    pub keys: KeysConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub default_columns: u8,
    pub detail_panel_width: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub root_dirs: Vec<String>,
    pub scan_depth: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub shell: String,
    pub cwd_poll_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    pub exit_insert: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
            scan: ScanConfig::default(),
            terminal: TerminalConfig::default(),
            keys: KeysConfig::default(),
        }
    }
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            default_columns: 2,
            detail_panel_width: 45,
        }
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            root_dirs: vec!["~/workplace".to_string()],
            scan_depth: 2,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        let shell = std::env::var("SHELL")
            .unwrap_or_else(|_| "/bin/zsh".to_string());
        Self {
            shell,
            cwd_poll_interval: 2,
        }
    }
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            exit_insert: "ctrl-]".to_string(),
        }
    }
}

impl Config {
    /// Returns the default config file path.
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("termgrid")
            .join("config.toml")
    }

    /// Loads config from a file, returns default config if file doesn't exist or parse fails.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                toml::from_str(&content)
                    .unwrap_or_else(|_| Config::default())
            }
            Err(_) => Config::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.layout.default_columns, 2);
        assert_eq!(config.layout.detail_panel_width, 45);
        assert_eq!(config.scan.root_dirs, vec!["~/workplace"]);
        assert_eq!(config.scan.scan_depth, 2);
        assert_eq!(config.terminal.cwd_poll_interval, 2);
        assert_eq!(config.keys.exit_insert, "ctrl-]");
    }

    #[test]
    fn test_parse_partial_toml() {
        let toml_str = r#"
[layout]
default_columns = 3

[keys]
exit_insert = "ctrl-q"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.layout.default_columns, 3);
        assert_eq!(config.layout.detail_panel_width, 45); // default
        assert_eq!(config.scan.scan_depth, 2); // default
        assert_eq!(config.keys.exit_insert, "ctrl-q");
    }

    #[test]
    fn test_parse_empty_toml() {
        let toml_str = "";
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.layout.default_columns, 2);
        assert_eq!(config.scan.scan_depth, 2);
        assert_eq!(config.keys.exit_insert, "ctrl-]");
    }

    #[test]
    fn test_load_nonexistent_file() {
        let config = Config::load(Path::new("/nonexistent/path/config.toml"));
        assert_eq!(config.layout.default_columns, 2);
        assert_eq!(config.scan.scan_depth, 2);
    }

    #[test]
    fn test_columns_clamped() {
        let config = Config::default();
        assert!(config.layout.default_columns >= 1);
        assert!(config.layout.default_columns <= 3);
    }

    #[test]
    fn test_config_path() {
        let path = Config::config_path();
        assert!(path.to_string_lossy().ends_with("termgrid/config.toml"));
    }
}
