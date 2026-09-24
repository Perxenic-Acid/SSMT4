//! File ownership diagnosis. Never unload DLLs or close handles in another process.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockOwner {
    pub pid: u32,
    pub name: String,
    pub executable: String,
    pub started: String,
    pub can_terminate: bool,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockReport {
    pub paths: Vec<String>,
    pub owners: Vec<LockOwner>,
    pub warnings: Vec<String>,
}

fn collect(path: &Path, paths: &mut Vec<PathBuf>, warnings: &mut Vec<String>, follow_root: bool) {
    if paths.len() >= 20000 {
        if !warnings.iter().any(|s| s.contains("20000")) {
            warnings.push("目录超过 20000 项，扫描结果不完整，请缩小范围。".into());
        }
        return;
    }
    paths.push(path.to_owned());
    // Resolve a selected linked Mod, but avoid descending into nested link cycles.
    if path.is_dir() {
        if !follow_root
            && path
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(true)
        {
            warnings.push(format!("未递归检查链接目录：{}", path.display()));
            return;
        }
        match std::fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => collect(&entry.path(), paths, warnings, false),
                        Err(e) => warnings.push(e.to_string()),
                    }
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", path.display())),
        }
    }
}

fn scan(paths: Vec<String>) -> Result<LockReport, String> {
    let mut report = LockReport {
        paths: paths.clone(),
        ..Default::default()
    };
    let mut expanded = Vec::new();
    for path in paths {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("必须使用绝对路径".into());
        }
        if !path.exists() {
            report
                .warnings
                .push(format!("文件不存在或无法访问：{}", path.display()));
            continue;
        }
        collect(&path, &mut expanded, &mut report.warnings, true);
    }
    platform::scan(&expanded, &mut report)?;
    report.owners.sort_by_key(|p| p.pid);
    report.owners.dedup_by_key(|p| p.pid);
    Ok(report)
}

#[tauri::command]
pub async fn query_file_locks(paths: Vec<String>) -> Result<LockReport, String> {
    tauri::async_runtime::spawn_blocking(move || scan(paths))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn terminate_file_lock_owners(
    paths: Vec<String>,
    approved: Vec<LockOwner>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Recheck ownership and creation time after the user has read the dialog.
        let current = scan(paths)?;
        for owner in approved {
            if let Some(current_owner) = current
                .owners
                .iter()
                .find(|p| p.pid == owner.pid && p.started == owner.started && p.can_terminate)
            {
                platform::terminate(current_owner)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn rename_mod_with_retry(
    app: tauri::AppHandle,
    source: String,
    destination: String,
    watch_root: String,
) -> Result<(), String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let source = PathBuf::from(source);
        let destination = PathBuf::from(destination);
        let root = PathBuf::from(watch_root);
        if source.parent() != destination.parent() || source == root || !source.starts_with(&root) {
            return Err("只支持 Mods 目录中的同级重命名".into());
        }
        if destination.exists() {
            return Err("目标目录已存在".into());
        }
        let state = app.state::<super::mod_manager::ModWatcher>();
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        use notify::Watcher;
        let paused = guard
            .as_mut()
            .map(|w| w.unwatch(&root).is_ok())
            .unwrap_or(false);
        let result = (|| {
            for attempt in 0..4 {
                if destination.exists() {
                    return Err("目标目录已存在".into());
                }
                match std::fs::rename(&source, &destination) {
                    Ok(()) => return Ok(()),
                    Err(e) if attempt < 3 && matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                        std::thread::sleep(std::time::Duration::from_millis(120 * (attempt + 1)));
                    }
                    Err(e) => {
                        return Err(if matches!(e.raw_os_error(), Some(5 | 32 | 33)) {
                            format!("FILE_LOCKED: {e}")
                        } else {
                            e.to_string()
                        })
                    }
                }
            }
            unreachable!()
        })();
        if paused {
            if let Some(watcher) = guard.as_mut() {
                if let Err(e) = watcher.watch(&root, notify::RecursiveMode::Recursive) {
                    // A completed rename must not be reported as failed and retried.
                    eprintln!("Failed to resume Mods watcher: {e}");
                }
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn scan(_: &[PathBuf], _: &mut LockReport) -> Result<(), String> {
        Err("占用诊断仅支持 Windows".into())
    }
    pub fn terminate(_: &LockOwner) -> Result<(), String> {
        Err("仅支持 Windows".into())
    }
}

#[cfg(windows)]
#[path = "file_locks_windows.rs"]
mod platform;
