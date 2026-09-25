use std::env;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub struct PathManager;

impl PathManager {
    /// SSMT 安装目录（可执行文件所在目录）。
    /// 打包/安装后即为安装根目录；开发环境下回退到当前工作目录。
    pub fn ssmt_install_dir() -> PathBuf {
        if let Ok(exe_path) = env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                return exe_dir.to_path_buf();
            }
        }

        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    /// 默认缓存目录：安装目录下的 SSMT4CachedFolder。
    /// 替代旧的 AppData 默认位置，用户升级到新版本后仍可随时在设置中修改。
    pub fn ssmt_default_cache_folder() -> PathBuf {
        Self::ssmt_install_dir().join("SSMT4CachedFolder")
    }

    /// 当前生效的缓存根目录：
    /// 1) 优先读取设置文件中的 DBMTWorkFolder（用户显式选择过的位置）；
    /// 2) 设置未配置时使用安装目录下的默认位置；
    /// 3) 极端情况下回退到 AppData，避免后端落盘失败。
    pub fn ssmt_cache_root(app: &AppHandle) -> PathBuf {
        if let Ok(settings_dir) = app.path().app_local_data_dir() {
            let settings_path = settings_dir.join("SSMT4GlobalConfigs").join("settings.json");
            if let Ok(raw) = std::fs::read_to_string(settings_path) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(folder) = value
                        .get("DBMTWorkFolder")
                        .and_then(|folder| folder.as_str())
                    {
                        let folder = folder.trim();
                        if !folder.is_empty() {
                            return PathBuf::from(folder);
                        }
                    }
                }
            }
        }

        let default = Self::ssmt_default_cache_folder();
        if default.exists() {
            return default;
        }

        app.path()
            .app_local_data_dir()
            .map(|dir| dir.join("SSMT4CachedFolder"))
            .unwrap_or(default)
    }

    /// SSMT 自带的 resources 文件夹路径（优先运行目录旁的 resources，找不到则回退到开发目录）。
    pub fn ssmt_resources_folder() -> PathBuf {
        // 1) 尝试可执行文件所在目录/父目录的 resources（适用于打包/安装后的路径）
        if let Ok(exe_path) = env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let candidate = exe_dir.join("resources");
                if candidate.exists() {
                    return candidate;
                }

                // 某些打包方式下，resources 可能与 exe 同级的上一级目录
                if let Some(parent) = exe_dir.parent() {
                    let candidate = parent.join("resources");
                    if candidate.exists() {
                        return candidate;
                    }
                }
            }
        }

        // 2) 开发环境回退：使用当前工作目录下的 src-tauri/resources
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("src-tauri")
            .join("resources")
    }

    ///SSMT自带的resources目录下的Games文件夹
    pub fn ssmt_games_folder() -> PathBuf {
        Self::ssmt_resources_folder().join("Games")
    }

    ///SSMT自带的resources目录下的GameType文件夹
    ///只作为安装包内的初始来源，启动时会同步到用户配置目录。
    pub fn bundled_gametype_folder() -> PathBuf {
        Self::ssmt_resources_folder().join("GameType")
    }

    pub fn app_data_local_folder() -> PathBuf {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local);
        }

        // 最后兜底：当前工作目录
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    pub fn ssmt_global_config_folder() -> PathBuf {
        Self::app_data_local_folder().join("SSMT4GlobalConfigs")
    }

    /// 应用级插件包目录。插件包与第三方外部依赖分开存放。
    pub fn ssmt_plugins_folder() -> PathBuf {
        Self::ssmt_global_config_folder().join("Plugins")
    }

    pub fn global_config_games_folder() -> PathBuf {
        Self::ssmt_global_config_folder().join("Games")
    }

    ///用户配置目录下的GameType文件夹
    ///与Games文件夹一样位于SSMT4GlobalConfigs下，是运行时唯一的数据类型读取来源。
    pub fn ssmt_gametype_folder() -> PathBuf {
        Self::ssmt_global_config_folder().join("GameType")
    }

    pub fn global_config_games_game_folder(game_name: &str) -> PathBuf {
        Self::global_config_games_folder().join(game_name)
    }
}

/// 返回 SSMT 安装目录（可执行文件所在目录），供前端计算默认缓存位置。
#[tauri::command]
pub fn ssmt_install_directory() -> String {
    PathManager::ssmt_install_dir().to_string_lossy().to_string()
}
