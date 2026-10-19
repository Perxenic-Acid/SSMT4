use std::collections::BTreeMap;
use std::{fs, path::Path};

pub const COMPONENT_ALIASES_FILE: &str = "ComponentAliases.json";
pub type ComponentAliases = BTreeMap<String, String>;

pub fn valid_component_aliases(aliases: &ComponentAliases) -> bool {
    aliases.len() <= 4096
        && aliases.iter().all(|(key, alias)| {
            !key.is_empty()
                && key.len() <= 128
                && key.as_bytes()[0].is_ascii_alphanumeric()
                && key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                && alias.chars().count() <= 128
                && !alias.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
        })
}

pub fn read_component_aliases(lod: &Path) -> Result<ComponentAliases, String> {
    let path = lod.join(COMPONENT_ALIASES_FILE);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("无法读取 {}: {error}", path.display())),
    };
    let aliases: ComponentAliases = serde_json::from_str(&content)
        .map_err(|error| format!("{} 格式错误: {error}", path.display()))?;
    if !valid_component_aliases(&aliases) {
        return Err(format!(
            "{} 包含无效的 Component 标识或别名",
            path.display()
        ));
    }
    Ok(aliases
        .into_iter()
        .filter_map(|(key, value)| {
            let alias = value.trim().to_string();
            (!alias.is_empty()).then_some((key, alias))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_metadata_defaults_to_no_aliases_and_rejects_unsafe_new_values() {
        let lod: crate::workspace_access::models::PortableLodV1 =
            serde_json::from_str(r#"{"name":"LOD0","drawIB":[],"skipIB":[],"vsCheck":[]}"#)
                .unwrap();
        assert!(lod.component_aliases.is_empty());
        assert!(serde_json::to_value(&lod)
            .unwrap()
            .get("componentAliases")
            .is_none());
        for (key, alias) in [
            ("../bad", "Body"),
            ("mesh", "line\nbreak"),
            ("mesh", &"x".repeat(129)),
        ] {
            assert!(!valid_component_aliases(&BTreeMap::from([(
                key.to_string(),
                alias.to_string()
            )])));
        }
        assert!(valid_component_aliases(&BTreeMap::from([(
            "848c5980-3-0".to_string(),
            "😀".repeat(128)
        )])));
    }
}
