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
    /// 旧包只补展示信息，不改写磁盘清单、来源信任或实际依赖检查。
    pub(super) fn display_dependencies(&self) -> Vec<super::ExternalDependency> {
        let mut dependencies = self.external_dependencies.clone();
        for dependency in &mut dependencies {
            if self.id == super::hoyoshade::HOYOSHADE_PLUGIN_ID
                && matches!(self.version.as_str(), "0.1.0" | "0.1.1")
                && dependency.id == super::hoyoshade::HOYOSHADE_DEPENDENCY_ID
            {
                dependency
                    .download_url
                    .get_or_insert_with(|| "https://github.com/DuolaD/HoYoShade/releases".into());
                dependency.download_hint.get_or_insert_with(|| {
                    "下载并解压 HoYoShade，按发布说明补齐效果库，再选择包含所列文件的目录。".into()
                });
            }
            if matches!(self.version.as_str(), "0.2.1" | "0.2.2")
                && super::dlss5::is_optional_integration_dependency(
                    &self.id,
                    &self.version,
                    &dependency.id,
                )
            {
                dependency
                    .required_for
                    .get_or_insert_with(|| vec!["external-manager".into()]);
                let hint = match dependency.id.as_str() {
                    "dlss5-swapper" => "仅用于旧外部管理器；原下载源当前不可用。热注入无需此工具，请使用上方组件配置入口。",
                    "dlss5-adapter" => "仅用于旧外部管理器；SSMT 专用适配器尚未独立发布。热注入无需此依赖。",
                    _ => "仅用于旧外部管理器；热注入所需组件可从上方入口按显卡下载，无需单独配置此目录。",
                };
                dependency.download_hint.get_or_insert_with(|| hint.into());
            }
        }
        dependencies
    }

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
    fn legacy_dependency_display_is_read_only_and_does_not_invent_downloads() {
        let shade = PluginManifest::from_json_str(include_str!(
            "../../../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.1.0/ssmt-plugin.json"
        ))
        .unwrap();
        let original = shade.clone();
        assert_eq!(
            shade.display_dependencies()[0].download_url.as_deref(),
            Some("https://github.com/DuolaD/HoYoShade/releases")
        );
        assert_eq!(shade, original);
        let mut other = shade;
        other.id = "example.other".into();
        assert!(other.display_dependencies()[0].download_url.is_none());

        let dlss = PluginManifest::from_json_str(include_str!("../../../native/official-plugin-packages/ssmt.dlss5.integration/0.2.1/ssmt-plugin.json")).unwrap();
        for dependency in dlss.display_dependencies() {
            assert_eq!(
                dependency.required_for,
                Some(vec!["external-manager".into()])
            );
            assert!(dependency.download_url.is_none());
            assert!(dependency.download_hint.is_some());
        }
        let mut cold = dlss;
        cold.version = "0.2.0".into();
        assert!(cold
            .display_dependencies()
            .iter()
            .all(|dependency| dependency.download_hint.is_none()));

        for raw in [include_str!("../../../native/official-plugin-packages/ssmt.hoyoshade.bridge/0.1.1/ssmt-plugin.json"), include_str!("../../../native/official-plugin-packages/ssmt.dlss5.integration/0.2.2/ssmt-plugin.json")] {
            let manifest = PluginManifest::from_json_str(raw).unwrap();
            assert_eq!(manifest.external_dependencies, manifest.display_dependencies(), "新包应自行提供完整下载说明");
        }
    }

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
