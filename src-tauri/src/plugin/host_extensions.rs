//! 本体侧贡献只声明已注册能力与数据文件，不把包路径当作可执行代码。
use super::{PluginManifest, PluginManifestError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostExtensionContribution {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub config: Option<String>,
    #[serde(default)]
    pub settings_component: Option<String>,
}

impl HostExtensionContribution {
    pub(super) fn validate(&self) -> Result<(), PluginManifestError> {
        super::validate_plugin_id(&self.id)?;
        super::validate_plugin_id(&self.provider)?;
        if let Some(path) = &self.config {
            super::validate_package_relative_path(path)?;
        }
        if let Some(component) = &self.settings_component {
            super::validate_plugin_id(component)?;
        }
        Ok(())
    }
}

impl PluginManifest {
    /// 不指定 requiredFor 的旧依赖仍为必需；显式空列表表示仅在手动使用时检查。
    pub(super) fn requires_dependency(&self, id: &str, feature: &str) -> bool {
        match self
            .external_dependencies
            .iter()
            .find(|item| item.id == id)
            .and_then(|dependency| dependency.required_for.as_ref())
        {
            Some(features) => features.iter().any(|item| item == feature),
            // 兼容旧包时只在此适配，不让通用启动协调器了解特定插件版本。
            None => !super::dlss5::is_optional_integration_dependency(&self.id, &self.version, id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditions_are_declared_and_legacy_dependencies_stay_required() {
        let manifest = PluginManifest::from_json_str(r#"{
            "schemaVersion":1,"id":"example.extension","name":"Example","version":"1.0.0","author":"Example",
            "compatibility":{"ssmt":">=4.0.0","platforms":["windows-x64"]},
            "contributions":{"hostExtensions":[{"id":"graphics","provider":"example.graphics","config":"host/graphics.json","settingsComponent":"example.settings"}]},
            "externalDependencies":[{"id":"always","type":"directory"},{"id":"manager","type":"directory","requiredFor":["external-manager"]},{"id":"optional","type":"directory","requiredFor":[]}]
        }"#).unwrap();
        assert!(manifest.requires_dependency("always", "launch"));
        assert!(!manifest.requires_dependency("manager", "launch"));
        assert!(manifest.requires_dependency("manager", "external-manager"));
        assert!(!manifest.requires_dependency("optional", "launch"));
        let mut invalid = manifest;
        invalid.contributions.host_extensions[0].config = Some("../outside.json".into());
        assert!(invalid.validate().is_err());
    }
}
