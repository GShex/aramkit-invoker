use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Directory name used below the platform config directory.
pub const APP_DIR_NAME: &str = "aramkit-invoker";

/// Persistent settings for the tray helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path to the ARAMKit desktop executable.
    pub aramkit_app_path: Option<PathBuf>,
    pub tcls_path: Option<PathBuf>,
    pub wegame_path: Option<PathBuf>,
    pub auto_launch_game: Option<GameLauncher>,
    pub browser_window: Option<WindowBounds>,
    pub main_window: Option<WindowBounds>,
    pub auto_launch_aramkit: bool,
    pub browser_open_homepage: bool,
    pub browser_refresh_on_selection: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            aramkit_app_path: None,
            tcls_path: None,
            wegame_path: None,
            auto_launch_game: None,
            browser_window: None,
            main_window: None,
            auto_launch_aramkit: true,
            browser_open_homepage: false,
            browser_refresh_on_selection: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameLauncher {
    Tcls,
    WeGame,
}

impl GameLauncher {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcls => "TCLS 客户端",
            Self::WeGame => "WeGame",
        }
    }
}

impl Config {
    /// Loads the config. A missing file is not an error; it simply yields defaults.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).map_err(|err| format!("配置文件格式错误：{err}"))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!("无法读取配置文件：{err}")),
        }
    }

    /// Writes the config atomically through a temporary file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().ok_or_else(|| "配置文件路径无效".to_owned())?;
        fs::create_dir_all(parent).map_err(|err| format!("无法创建配置目录：{err}"))?;

        let json =
            serde_json::to_vec_pretty(self).map_err(|err| format!("无法序列化配置：{err}"))?;

        // Create the temporary file in the destination directory so the final
        // rename/persist stays on the same volume and the name is unpredictable.
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|err| format!("无法创建临时配置文件：{err}"))?;
        temp.write_all(&json)
            .map_err(|err| format!("无法写入配置文件：{err}"))?;
        temp.as_file()
            .sync_all()
            .map_err(|err| format!("无法同步配置文件：{err}"))?;
        temp.persist(path)
            .map_err(|err| format!("无法保存配置文件：{}", err.error))?;
        Ok(())
    }
}

/// Returns `%APPDATA%\aramkit-invoker\config.json` on Windows.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR_NAME).join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_loads_defaults() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("missing.json");

        assert_eq!(Config::load(&path).expect("load"), Config::default());
    }

    #[test]
    fn older_settings_keep_existing_startup_and_refresh_behavior() {
        let config: Config =
            serde_json::from_str(r#"{"aramkit_app_path":null}"#).expect("old config");
        assert!(config.auto_launch_aramkit);
        assert!(!config.browser_open_homepage);
        assert!(!config.browser_refresh_on_selection);
    }

    #[test]
    fn config_round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested").join("config.json");
        let config = Config {
            aramkit_app_path: Some(PathBuf::from(r"C:\Apps\ARAMKit\ARAMKit.exe")),
            browser_window: Some(WindowBounds {
                x: -1200,
                y: 80,
                width: 1000,
                height: 720,
            }),
            main_window: Some(WindowBounds {
                x: -1600,
                y: 120,
                width: 800,
                height: 900,
            }),
            auto_launch_aramkit: false,
            browser_open_homepage: true,
            browser_refresh_on_selection: true,
            ..Config::default()
        };

        config.save(&path).expect("save");
        assert_eq!(Config::load(&path).expect("load"), config);
    }

    #[test]
    fn empty_path_is_preserved_as_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.json");
        let config = Config {
            aramkit_app_path: None,
            ..Config::default()
        };

        config.save(&path).expect("save");
        assert_eq!(Config::load(&path).expect("load"), config);
    }

    #[test]
    fn malformed_json_is_reported() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json").expect("write");

        let error = Config::load(&path).expect_err("must fail");
        assert!(error.contains("配置文件格式错误"));
    }
}
