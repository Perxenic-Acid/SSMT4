mod client;
mod engine;
mod manifest;
mod space;
pub(crate) mod task;
mod transaction;
mod transfer;

use crate::config::path_manager::PathManager;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Context {
    pub config_path: PathBuf,
    pub preset: String,
    pub config: Value,
    pub root: Option<PathBuf>,
}

impl Context {
    pub fn load(game_name: &str) -> Result<Self, String> {
        manifest::relative(game_name)?;
        if game_name.contains(['/', '\\']) {
            return Err("游戏实例名称无效".into());
        }
        let config_path =
            PathManager::global_config_games_game_folder(game_name).join("Config.json");
        let config: Value =
            serde_json::from_slice(&transaction::read_limited(&config_path, 8 * 1024 * 1024)?)
                .map_err(|e| format!("游戏实例配置无效：{e}"))?;
        let preset = config["gamePreset"]
            .as_str()
            .unwrap_or_default()
            .to_uppercase();
        if client::servers(&preset).is_empty() {
            return Err("当前游戏尚未支持米家安装管理".into());
        }
        let root = config["targetExePath"]
            .as_str()
            .filter(|s| !s.is_empty())
            .and_then(|s| Path::new(s).parent())
            .map(Path::to_path_buf);
        Ok(Self {
            config_path,
            preset,
            config,
            root,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    servers: Vec<client::Server>,
    current_server: Option<String>,
    selected_server: String,
    installed_version: Option<String>,
    client_found: bool,
    latest_version: String,
    pre_download_version: Option<String>,
    directory: Option<PathBuf>,
    target_exe: String,
    audio_languages: Vec<String>,
    recovery_pending: bool,
}

pub fn ini_values(root: &Path) -> Result<std::collections::BTreeMap<String, String>, String> {
    let path = transaction::safe(root, "config.ini")?;
    if !path.exists() {
        return Ok(Default::default());
    }
    let bytes = transaction::read_limited(&path, 1024 * 1024)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "客户端 config.ini 不是 UTF-8，请通过官方启动器修复")?;
    let mut general = false;
    let mut sections = 0;
    let mut values = std::collections::BTreeMap::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') {
            general = line.eq_ignore_ascii_case("[General]");
            if general {
                sections += 1;
            }
        } else if general {
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_lowercase();
                if values.insert(key, value.trim().into()).is_some() {
                    return Err("客户端 config.ini 存在重复配置项".into());
                }
            }
        }
    }
    if sections > 1 {
        return Err("客户端 config.ini 有多个 General 节".into());
    }
    Ok(values)
}

pub fn detect_server(preset: &str, root: &Path) -> Result<Option<String>, String> {
    let values = ini_values(root)?;
    if preset == "GIMI" {
        let cn = root.join("YuanShen.exe").is_file();
        let global = root.join("GenshinImpact.exe").is_file();
        if cn && global {
            return Err("目录同时含两种原神客户端，需先确认正在使用的版本".into());
        }
        if global {
            return Ok(Some("global".into()));
        }
        if !cn {
            return Ok(None);
        }
    } else if !client::servers(preset)
        .iter()
        .any(|s| root.join(&s.exe).is_file())
    {
        return Ok(None);
    }
    let biz = values
        .get("game_biz")
        .map(String::as_str)
        .unwrap_or_default();
    let channel = values
        .get("channel")
        .map(String::as_str)
        .unwrap_or_default();
    if channel == "14" {
        return Ok(Some("bilibili".into()));
    }
    if biz.ends_with("_global") {
        return Ok(Some("global".into()));
    }
    if channel == "1" && (preset == "GIMI" || biz.ends_with("_cn")) {
        return Ok(Some("cn".into()));
    }
    // 旧崩铁、绝区零国服与国际服使用相同 exe 和 channel，不能据此猜测国服。
    if let Some(uapc) = values
        .get("uapc")
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    {
        if uapc
            .as_object()
            .is_some_and(|o| o.keys().any(|k| k.ends_with("_global")))
        {
            return Ok(Some("global".into()));
        }
        if uapc
            .as_object()
            .is_some_and(|o| o.keys().any(|k| k.ends_with("_cn")))
        {
            return Ok(Some("cn".into()));
        }
    }
    Ok(None)
}

#[tauri::command]
pub async fn hoyoplay_estimate(request: engine::Request) -> Result<space::Estimate, String> {
    tokio::task::spawn_blocking(move || space::estimate(request))
        .await
        .map_err(|e| format!("空间预估任务失败：{e}"))?
}

#[tauri::command]
pub fn hoyoplay_start(request: engine::Request) -> Result<task::Snapshot, String> {
    let task = task::Task::new(request.game_name.clone(), request.directory.clone());
    task.snapshot.lock().map_err(|e| e.to_string())?.operation = request.operation.clone();
    task::reserve(task.clone())?;
    let initial = task.snapshot();
    tauri::async_runtime::spawn_blocking(move || {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine::run(request, &task)))
                .unwrap_or_else(|_| {
                    Err("后台维护任务异常终止；若已开始文件提交，请恢复原客户端".into())
                });
        if let Err(e) = &result {
            eprintln!("[HoYoPlay] 米家游戏维护：{e}");
        }
        task.finish(result);
    });
    Ok(initial)
}

#[tauri::command]
pub async fn hoyoplay_check_launch(game_name: String) -> Result<(), String> {
    task::assert_launch_allowed(Some(&game_name))?;
    tokio::task::spawn_blocking(move || {
        let context = Context::load(&game_name)?;
        if context
            .root
            .is_some_and(|root| root.join(".ssmt-maintenance/journal.json").exists())
        {
            return Err("客户端文件提交曾中断，请在游戏设置 → 游戏管理中恢复原客户端".into());
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn hoyoplay_status(
    game_name: String,
    server_id: Option<String>,
    home_only: Option<bool>,
    local_only: Option<bool>,
) -> Result<Status, String> {
    tokio::task::spawn_blocking(move || {
        let context = Context::load(&game_name)?;
        let current_server = context
            .root
            .as_deref()
            .map(|p| detect_server(&context.preset, p))
            .transpose()?
            .flatten();
        let selected_server = server_id
            .or_else(|| current_server.clone())
            .unwrap_or_else(|| "cn".into());
        let servers = client::servers(&context.preset);
        let server = servers
            .iter()
            .find(|s| s.id == selected_server)
            .ok_or("未知游戏渠道")?;
        let remote = if local_only.unwrap_or(false) {
            client::cached_status_branches(server)
        } else {
            Some(client::status_branches(server)?)
        };
        let local = context
            .root
            .as_deref()
            .map(ini_values)
            .transpose()?
            .unwrap_or_default();
        let client_found = context
            .root
            .as_ref()
            .is_some_and(|r| servers.iter().any(|s| r.join(&s.exe).is_file()));
        let installed_version = local.get("game_version").cloned().filter(|_| client_found);
        if let Some(version) = &installed_version {
            client::version(version)?;
        }
        let mut audio_languages = context.config["hoyoplayAudioLanguages"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| client::AUDIO.contains(s))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_else(|| vec!["zh-cn".into()]);
        if let Some(root) = context
            .root
            .as_ref()
            .filter(|_| !home_only.unwrap_or(false) && !local_only.unwrap_or(false))
        {
            let launch = client::launch_config(client::metadata_http()?, server)?;
            audio_languages = installed_audio(root, &launch, &audio_languages)?;
        }
        let recovery_pending = context
            .root
            .as_ref()
            .is_some_and(|root| root.join(".ssmt-maintenance/journal.json").exists());
        Ok(Status {
            target_exe: server.exe.clone(),
            servers,
            current_server,
            selected_server,
            installed_version,
            client_found,
            latest_version: remote
                .as_ref()
                .map(|(main, _)| main.tag.clone())
                .unwrap_or_default(),
            pre_download_version: remote.and_then(|(_, pre)| pre.map(|b| b.tag)),
            directory: context.root,
            audio_languages,
            recovery_pending,
        })
    })
    .await
    .map_err(|e| format!("版本查询任务失败：{e}"))?
}

pub fn installed_audio(
    root: &Path,
    launch: &Value,
    requested: &[String],
) -> Result<Vec<String>, String> {
    let mut result = requested.to_vec();
    for field in ["audio_pkg_scan_dir", "audio_pkg_config_dir"] {
        if let Some(name) = launch[field].as_str().filter(|s| !s.is_empty()) {
            let path = transaction::safe(root, name)?;
            if !path.exists() {
                continue;
            }
            let bytes = transaction::read_limited(&path, 1024 * 1024)?;
            if bytes.len() > 1024 * 1024 {
                return Err("语音记录文件异常大".into());
            }
            let content = String::from_utf8(bytes).map_err(|_| "语音记录文件无法解析")?;
            for (code, name) in [
                ("zh-cn", "Chinese"),
                ("en-us", "English"),
                ("ja-jp", "Japanese"),
                ("ko-kr", "Korean"),
            ] {
                if content.contains(name) || content.contains(code) {
                    result.push(code.into());
                }
            }
        }
    }
    result.sort();
    result.dedup();
    Ok(result)
}

#[cfg(test)]
mod tests;
