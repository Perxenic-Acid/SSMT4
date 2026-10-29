//! DLSS5 的游戏路线和组件基线由包提供；旧包使用冻结的兼容基线。
use super::{
    gimi_hot_runtime::{checked_path, read_text},
    registry::PluginRegistry,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Route {
    pub preset: String,
    pub executables: Vec<String>,
    pub route: String,
    pub api: String,
    #[serde(default)]
    pub required_launch_args: Vec<String>,
    #[serde(default)]
    pub excluded_launch_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Policy {
    schema_version: u32,
    assets: String,
    gimi_assets: String,
    routes: Vec<Route>,
    #[serde(skip)]
    root: PathBuf,
}

impl Policy {
    pub(super) fn from_registry(registry: &PluginRegistry) -> Result<Option<Self>, String> {
        let Some(plugin) = registry.find(super::dlss5::DLSS5_PLUGIN_ID) else {
            return Ok(None);
        };
        let Some(config) = plugin
            .manifest
            .contributions
            .host_extensions
            .iter()
            .find(|item| item.provider == "ssmt.graphics.dlss5")
            .and_then(|item| item.config.as_deref())
        else {
            return Ok(None);
        };
        let root = super::gimi_hot_runtime::canonical_path(&plugin.package_root)
            .map_err(|e| e.to_string())?;
        let path = checked_path(&root, config)?;
        let mut policy: Self = serde_json::from_str(&read_text(&path)?)
            .map_err(|e| format!("DLSS5 路线声明无效：{e}"))?;
        policy.root = root;
        policy.validate()?;
        Ok(Some(policy))
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.routes.is_empty() {
            return Err("不支持的 DLSS5 路线声明".into());
        }
        super::validate_package_relative_path(&self.assets).map_err(|e| e.to_string())?;
        super::validate_package_relative_path(&self.gimi_assets).map_err(|e| e.to_string())?;
        let mut targets = std::collections::HashSet::new();
        for route in &self.routes {
            if !["feeder", "native-bridge", "native"].contains(&route.route.as_str())
                || !["d3d11", "d3d12"].contains(&route.api.as_str())
                || route.executables.is_empty()
                || ((route.route == "native") != (route.api == "d3d12"))
            {
                return Err("DLSS5 路线或图形 API 无效".into());
            }
            for flag in route
                .required_launch_args
                .iter()
                .chain(&route.excluded_launch_args)
            {
                if flag.len() < 2
                    || !flag.starts_with('-')
                    || !flag[1..]
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                {
                    return Err("DLSS5 启动参数声明无效".into());
                }
            }
            for executable in &route.executables {
                super::validate_package_relative_path(executable).map_err(|e| e.to_string())?;
                if executable.contains(['/', '\\'])
                    || !targets.insert((
                        route.preset.to_ascii_lowercase(),
                        executable.to_ascii_lowercase(),
                    ))
                {
                    return Err("DLSS5 目标重复或文件名无效".into());
                }
            }
        }
        Ok(())
    }

    pub(super) fn select(&self, preset: &str, executable: &Path) -> Result<&Route, String> {
        let name = executable
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or_default();
        self.routes
            .iter()
            .find(|route| {
                route.preset.eq_ignore_ascii_case(preset.trim())
                    && route
                        .executables
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(name))
            })
            .ok_or_else(|| "插件未声明当前游戏预设与目标程序的 DLSS5 路线".into())
    }

    pub(super) fn assets(&self, gimi: bool) -> Result<String, String> {
        read_text(&checked_path(
            &self.root,
            if gimi {
                &self.gimi_assets
            } else {
                &self.assets
            },
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_controls_routes_and_rejects_duplicates_and_paths() {
        for version in ["0.2.2", "0.2.3"] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../native/official-plugin-packages/ssmt.dlss5.integration/{version}"
            ));
            let mut policy: Policy = serde_json::from_str(
                &std::fs::read_to_string(root.join("host/graphics.json")).unwrap(),
            )
            .unwrap();
            policy.root = super::super::gimi_hot_runtime::canonical_path(&root).unwrap();
            policy.validate().unwrap();
            let route = policy
                .select("WWMI", Path::new("Client-Win64-Shipping.exe"))
                .unwrap();
            assert_eq!(route.required_launch_args, ["-dx11"]);
            assert!(policy.select("SRMI", Path::new("YuanShen.exe")).is_err());
            assert!(policy.assets(false).unwrap().contains("sha256"));
            policy.routes[0].executables.push("../outside.exe".into());
            assert!(policy.validate().is_err());
        }
    }
}
