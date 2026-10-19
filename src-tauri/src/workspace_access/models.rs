use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAccessPreflightRequest {
    pub workspace_path: String,
    pub game_preset: String,
    pub workspace_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAccessArchiveRequest {
    pub workspace_path: String,
    pub game_preset: String,
    pub workspace_name: String,
    pub output_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAccessPublishRequest {
    pub worker_url: String,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    pub workspace_path: String,
    pub game_preset: String,
    pub workspace_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attribution: PublicAttribution,
    #[serde(default)]
    pub captured_at: Option<String>,
    #[serde(default)]
    pub game_build: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub workspace_aliases: Option<Vec<String>>,
    #[serde(default)]
    pub archive_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicAttribution {
    #[serde(default = "anonymous_attribution")]
    pub mode: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

fn anonymous_attribution() -> String {
    "anonymous".to_string()
}

impl Default for PublicAttribution {
    fn default() -> Self {
        Self {
            mode: anonymous_attribution(),
            display_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePublishResult {
    pub submission_id: String,
    pub entry_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUploadProgress {
    pub submission_id: String,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub completed_parts: u32,
    pub total_parts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePreflightReport {
    pub valid: bool,
    pub errors: Vec<WorkspaceAccessIssue>,
    pub warnings: Vec<WorkspaceAccessIssue>,
    pub file_count: u64,
    pub total_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PortableWorkspaceMetadataV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAccessIssue {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl WorkspaceAccessIssue {
    pub fn describe(&self) -> String {
        format!(
            "{}: {} {}",
            self.code,
            self.path.as_deref().unwrap_or(""),
            self.detail.as_deref().unwrap_or("")
        )
        .trim_end()
        .to_string()
    }

    pub fn new(code: impl Into<String>, path: Option<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            path,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableWorkspaceMetadataV1 {
    pub schema_version: u32,
    pub game_preset: String,
    pub workspace_name: String,
    pub lods: Vec<PortableLodV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableLodV1 {
    pub name: String,
    // 可选增量字段：旧发布和旧客户端都可继续读取 schema v1。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub component_aliases: crate::workspace::component_aliases::ComponentAliases,
    #[serde(rename = "drawIB", alias = "drawIb")]
    pub draw_ib: Vec<HashAlias>,
    #[serde(rename = "skipIB", alias = "skipIb")]
    pub skip_ib: Vec<SkipIbEntry>,
    pub vs_check: Vec<VsCheckEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashAlias {
    pub hash: String,
    pub alias: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipIbEntry {
    pub hash: String,
    pub alias: String,
    pub index_count: u64,
    pub first_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VsCheckEntry {
    pub enabled: bool,
    pub hash: String,
}
