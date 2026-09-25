use super::{ExternalDependency, PluginPermission, SUPPORTED_PLATFORM};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketplaceCatalog {
    pub schema_version: u32,
    pub entries: Vec<MarketplaceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketplaceEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    pub minimum_ssmt_version: String,
    #[serde(default)]
    pub maximum_ssmt_version: Option<String>,
    pub supported_platforms: Vec<String>,
    #[serde(default)]
    pub supported_games: Vec<String>,
    pub package_size: u64,
    #[serde(default)]
    pub permissions: Vec<PluginPermission>,
    #[serde(default)]
    pub external_dependencies: Vec<ExternalDependency>,
    /// Third-party payloads must remain user-provided external dependencies.
    /// Keeping this field in the catalog makes an accidental redistribution
    /// visible and rejectable instead of silently treating it as an adapter.
    #[serde(default)]
    pub bundled_third_party_payloads: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceError {
    UnsupportedSchema(u32),
    InvalidEntry(String),
    DuplicateEntry(String),
    InvalidUrl(String),
    InvalidHash(String),
    UnsupportedPlatform(String),
    BundledThirdPartyPayload(String),
}

impl std::fmt::Display for MarketplaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported marketplace schema: {version}")
            }
            Self::InvalidEntry(detail) => write!(formatter, "invalid marketplace entry: {detail}"),
            Self::DuplicateEntry(id) => write!(formatter, "duplicate marketplace entry: {id}"),
            Self::InvalidUrl(url) => {
                write!(formatter, "marketplace download URL must use HTTPS: {url}")
            }
            Self::InvalidHash(hash) => write!(formatter, "invalid SHA256: {hash}"),
            Self::UnsupportedPlatform(platform) => {
                write!(formatter, "unsupported marketplace platform: {platform}")
            }
            Self::BundledThirdPartyPayload(payload) => write!(
                formatter,
                "marketplace packages must not bundle third-party payload: {payload}"
            ),
        }
    }
}

impl std::error::Error for MarketplaceError {}

impl MarketplaceCatalog {
    pub fn validate(&self) -> Result<(), MarketplaceError> {
        if self.schema_version != 1 {
            return Err(MarketplaceError::UnsupportedSchema(self.schema_version));
        }
        let mut ids = HashSet::new();
        for entry in &self.entries {
            if !ids.insert(format!("{}@{}", entry.id, entry.version)) {
                return Err(MarketplaceError::DuplicateEntry(format!(
                    "{}@{}",
                    entry.id, entry.version
                )));
            }
            if entry.id.trim().is_empty()
                || entry.name.trim().is_empty()
                || entry.author.trim().is_empty()
            {
                return Err(MarketplaceError::InvalidEntry(
                    "id, name and author are required".to_string(),
                ));
            }
            semver::Version::parse(&entry.version).map_err(|_| {
                MarketplaceError::InvalidEntry(format!("invalid version: {}", entry.version))
            })?;
            semver::VersionReq::parse(&entry.minimum_ssmt_version.replace(['x', 'X', '*'], "0"))
                .map_err(|_| {
                    MarketplaceError::InvalidEntry(format!(
                        "invalid minimum SSMT version: {}",
                        entry.minimum_ssmt_version
                    ))
                })?;
            if let Some(maximum) = &entry.maximum_ssmt_version {
                semver::VersionReq::parse(&format!("<={}", maximum.replace(['x', 'X', '*'], "0")))
                    .map_err(|_| {
                        MarketplaceError::InvalidEntry(format!(
                            "invalid maximum SSMT version: {maximum}"
                        ))
                    })?;
            }
            let url = reqwest::Url::parse(&entry.download_url)
                .map_err(|_| MarketplaceError::InvalidUrl(entry.download_url.clone()))?;
            if url.scheme() != "https" {
                return Err(MarketplaceError::InvalidUrl(entry.download_url.clone()));
            }
            if entry.sha256.len() != 64
                || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(MarketplaceError::InvalidHash(entry.sha256.clone()));
            }
            if entry.supported_platforms.is_empty() {
                return Err(MarketplaceError::InvalidEntry(
                    "supportedPlatforms is empty".to_string(),
                ));
            }
            for platform in &entry.supported_platforms {
                if platform != SUPPORTED_PLATFORM {
                    return Err(MarketplaceError::UnsupportedPlatform(platform.clone()));
                }
            }
            if let Some(payload) = entry
                .bundled_third_party_payloads
                .iter()
                .find(|payload| !payload.trim().is_empty())
            {
                return Err(MarketplaceError::BundledThirdPartyPayload(payload.clone()));
            }
        }
        Ok(())
    }

    pub fn from_json_str(raw: &str) -> Result<Self, MarketplaceError> {
        let catalog = serde_json::from_str::<Self>(raw)
            .map_err(|error| MarketplaceError::InvalidEntry(error.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }
}

impl MarketplaceEntry {
    pub fn supports(&self, ssmt_version: &str, platform: &str, game: Option<&str>) -> bool {
        let Ok(version) = semver::Version::parse(ssmt_version) else {
            return false;
        };
        let minimum =
            semver::VersionReq::parse(&self.minimum_ssmt_version.replace(['x', 'X', '*'], "0"));
        if minimum.map_or(true, |requirement| !requirement.matches(&version)) {
            return false;
        }
        if let Some(maximum) = &self.maximum_ssmt_version {
            let Ok(requirement) =
                semver::VersionReq::parse(&format!("<={}", maximum.replace(['x', 'X', '*'], "0")))
            else {
                return false;
            };
            if !requirement.matches(&version) {
                return false;
            }
        }
        self.supported_platforms.iter().any(|item| item == platform)
            && (game.is_none()
                || self.supported_games.is_empty()
                || self
                    .supported_games
                    .iter()
                    .any(|item| Some(item.as_str()) == game))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> MarketplaceEntry {
        MarketplaceEntry {
            id: "ssmt.hoyoshade.bridge".to_string(),
            name: "HoYoShade Bridge".to_string(),
            description: "External launcher integration".to_string(),
            author: "SSMT".to_string(),
            version: "0.1.0".to_string(),
            download_url: "https://example.com/ssmt.hoyoshade.bridge.ssmtpkg".to_string(),
            sha256: "a".repeat(64),
            minimum_ssmt_version: ">=4.x".to_string(),
            maximum_ssmt_version: None,
            supported_platforms: vec!["windows-x64".to_string()],
            supported_games: vec!["GIMI".to_string()],
            package_size: 1234,
            permissions: vec![PluginPermission::ProcessSpawn],
            external_dependencies: Vec::new(),
            bundled_third_party_payloads: Vec::new(),
        }
    }

    #[test]
    fn accepts_curated_catalog_entry() {
        MarketplaceCatalog {
            schema_version: 1,
            entries: vec![entry()],
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn rejects_insecure_url_and_bad_hash() {
        let mut value = entry();
        value.download_url = "http://example.com/plugin.ssmtpkg".to_string();
        assert!(matches!(
            (MarketplaceCatalog {
                schema_version: 1,
                entries: vec![value]
            })
            .validate(),
            Err(MarketplaceError::InvalidUrl(_))
        ));
        let mut value = entry();
        value.sha256 = "bad".to_string();
        assert!(matches!(
            (MarketplaceCatalog {
                schema_version: 1,
                entries: vec![value]
            })
            .validate(),
            Err(MarketplaceError::InvalidHash(_))
        ));
    }

    #[test]
    fn rejects_duplicate_versions_and_unknown_platforms() {
        let first = entry();
        let second = first.clone();
        assert!(matches!(
            (MarketplaceCatalog {
                schema_version: 1,
                entries: vec![first, second]
            })
            .validate(),
            Err(MarketplaceError::DuplicateEntry(_))
        ));
        let mut value = entry();
        value.supported_platforms = vec!["linux-x64".to_string()];
        assert!(matches!(
            (MarketplaceCatalog {
                schema_version: 1,
                entries: vec![value]
            })
            .validate(),
            Err(MarketplaceError::UnsupportedPlatform(_))
        ));
    }

    #[test]
    fn rejects_bundled_third_party_payloads() {
        let mut value = entry();
        value.bundled_third_party_payloads = vec!["ReShade64.dll".to_string()];
        assert!(matches!(
            (MarketplaceCatalog {
                schema_version: 1,
                entries: vec![value]
            })
            .validate(),
            Err(MarketplaceError::BundledThirdPartyPayload(payload))
                if payload == "ReShade64.dll"
        ));
    }

    #[test]
    fn applies_minimum_maximum_platform_and_game_compatibility() {
        let mut value = entry();
        value.maximum_ssmt_version = Some("4.9.0".to_string());
        assert!(value.supports("4.1.89", "windows-x64", Some("GIMI")));
        assert!(!value.supports("5.0.0", "windows-x64", Some("GIMI")));
        assert!(!value.supports("4.1.89", "windows-x64", Some("ZZMI")));
    }
}
