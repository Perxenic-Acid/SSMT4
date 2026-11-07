use super::PluginPermission;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInjectedDll {
    pub plugin_id: String,
    pub plugin_name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInjectedDllsResult {
    pub dlls: Vec<PluginInjectedDll>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn plugin_launch_injected_dlls(
    game_name: String,
) -> Result<PluginInjectedDllsResult, String> {
    tauri::async_runtime::spawn_blocking(move || injected_dlls_for_game(&game_name))
        .await
        .map_err(|error| format!("后台读取额外注入 DLL 失败：{error}"))?
}

fn injected_dlls_for_game(game_name: &str) -> Result<PluginInjectedDllsResult, String> {
    if game_name.trim().is_empty() {
        return Ok(PluginInjectedDllsResult {
            dlls: Vec::new(),
            warnings: Vec::new(),
        });
    }
    super::data_store::publish_for_game(game_name)?;
    let registry = super::registry::PluginRegistry::from_default_location()
        .map_err(|error| error.to_string())?;
    let scoped = registry.for_game(game_name);
    let mut seen = HashSet::new();
    let mut dlls = Vec::new();
    let mut warnings = Vec::new();
    for plugin in scoped.selected_plugins().filter(|plugin| plugin.enabled) {
        if plugin.manifest.contributions.injected_dlls.is_empty() {
            continue;
        }
        if !plugin
            .manifest
            .permissions
            .iter()
            .any(|permission| *permission == PluginPermission::NativeInject)
        {
            warnings.push(format!(
                "插件 {} v{} 声明了额外注入 DLL，但缺少 native.inject 权限",
                plugin.manifest.name, plugin.manifest.version
            ));
            continue;
        }
        if !super::commands::has_native_injection_risk_ack(plugin) {
            warnings.push(format!(
                "插件 {} v{} 尚未接受原生注入风险与免责声明，已跳过注入",
                plugin.manifest.name, plugin.manifest.version
            ));
            continue;
        }
        for contribution in &plugin.manifest.contributions.injected_dlls {
            if let Err(error) = contribution.verify_file(&plugin.package_root) {
                warnings.push(format!(
                    "插件 {} v{} 的注入 DLL 校验失败：{error}",
                    plugin.manifest.name, plugin.manifest.version
                ));
                continue;
            }
            let path = plugin.package_root.join(&contribution.path);
            let key = path
                .to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase();
            if seen.insert(key) {
                dlls.push(PluginInjectedDll {
                    plugin_id: plugin.manifest.id.clone(),
                    plugin_name: plugin.manifest.name.clone(),
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
    }
    dlls.sort_by(|left, right| {
        left.path
            .to_ascii_lowercase()
            .cmp(&right.path.to_ascii_lowercase())
    });
    Ok(PluginInjectedDllsResult { dlls, warnings })
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_game_returns_no_injected_dlls() {
        let result = super::injected_dlls_for_game("").unwrap();
        assert!(result.dlls.is_empty());
        assert!(result.warnings.is_empty());
    }
}
