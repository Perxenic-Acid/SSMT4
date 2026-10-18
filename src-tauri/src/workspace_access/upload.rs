use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workspace_access::archive::{
    read_workspace_archive_metadata, validate_workspace_archive,
};
use crate::workspace_access::models::{
    PortableWorkspaceMetadataV1, WorkspaceAccessPublishRequest, WorkspacePublishResult,
    WorkspaceUploadProgress,
};
use crate::workspace_access::transfer::{sha256_file, workspace_access_http_client};
use crate::workspace_access::validate::preflight_workspace;

const PART_SIZE: u64 = 32 * 1024 * 1024;
const PART_BATCH_SIZE: usize = 100;
const UPLOAD_CONCURRENCY: usize = 3;
const UPLOAD_RETRIES: usize = 3;
// The completion request verifies the entire R2 object before publishing.  The
// reqwest default of 30 seconds is too short for ordinary large workspaces.
const SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);

static UPLOAD_CANCELLATIONS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionBody {
    metadata: PublicMetadataV1,
    archive: ArchiveDeclaration,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicMetadataV1 {
    schema_version: u32,
    game_preset: String,
    workspace_name: String,
    description: Option<String>,
    captured_at: Option<String>,
    game_build: Option<String>,
    attribution: crate::workspace_access::models::PublicAttribution,
    supersedes: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    workspace_aliases: Vec<String>,
    generator: Generator,
    lods: Vec<crate::workspace_access::models::PortableLodV1>,
    full_data: FullData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Generator {
    name: &'static str,
    version: &'static str,
    validator_version: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FullData {
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uncompressed_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_count: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ArchiveDeclaration {
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmissionResponse {
    submission_id: String,
    entry_id: String,
    status: String,
    metadata_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartsResponse {
    parts: Vec<PresignedPart>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresignedPart {
    part_number: u32,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: ErrorCode,
}

#[derive(Debug, Deserialize)]
struct ErrorCode {
    code: String,
    detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadState {
    version: u32,
    worker_url: String,
    archive_path: String,
    archive_sha256: String,
    archive_size: u64,
    part_size: u64,
    submission_id: String,
    entry_id: String,
    #[serde(default)]
    part_url_expires_at: Option<String>,
    completed_parts: BTreeMap<u32, String>,
}

pub async fn publish_workspace<F>(
    request: &WorkspaceAccessPublishRequest,
    on_progress: F,
) -> Result<WorkspacePublishResult, String>
where
    F: FnMut(WorkspaceUploadProgress),
{
    let worker_url = normalize_worker_url(&request.worker_url)?;
    let preflight = preflight_workspace(
        &crate::workspace_access::models::WorkspaceAccessPreflightRequest {
            workspace_path: request.workspace_path.clone(),
            game_preset: request.game_preset.clone(),
            workspace_name: request.workspace_name.clone(),
        },
    );
    let portable = preflight.metadata.ok_or_else(|| {
        preflight
            .errors
            .first()
            .map(|issue| issue.describe())
            .unwrap_or_else(|| "WORKSPACE_PREFLIGHT_FAILED".to_string())
    })?;
    let archive = request.archive_path.as_ref().map(PathBuf::from);
    let archive_info = match archive.as_deref() {
        Some(path) => Some(validate_archive_matches_workspace(path, &portable)?),
        None => None,
    };
    let metadata = make_public_metadata(request, portable, archive_info.as_ref());
    let client = workspace_service_client(request.proxy_port)?;

    if let Some(info) = archive_info {
        return publish_archive(&client, &worker_url, metadata, info, on_progress).await;
    }
    let response =
        initialize_submission(&client, &worker_url, metadata, 0, &new_idempotency_key()).await?;
    Ok(WorkspacePublishResult {
        submission_id: response.submission_id,
        entry_id: response.entry_id,
        status: response.status,
        metadata_path: response.metadata_path,
    })
}

pub async fn cancel_upload(
    worker_url: &str,
    archive_path: &Path,
    proxy_port: Option<u16>,
) -> Result<(), String> {
    let worker_url = normalize_worker_url(worker_url)?;
    request_upload_cancellation(archive_path);
    let state_path = upload_state_path(archive_path);
    let Some(state) = read_state(&state_path)? else {
        return Ok(());
    };
    if state.worker_url != worker_url {
        return Err("UPLOAD_STATE_SERVICE_MISMATCH".to_string());
    }
    let response = workspace_service_client(proxy_port)?
        .delete(format!(
            "{worker_url}/v1/submissions/{}",
            state.submission_id
        ))
        .send()
        .await
        .map_err(|_| "WORKSPACE_SERVICE_UNAVAILABLE")?;
    if !response.status().is_success() {
        return Err("UPLOAD_CANCEL_FAILED".to_string());
    }
    fs::remove_file(state_path).map_err(|_| "UPLOAD_STATE_DELETE_FAILED".to_string())
}

fn workspace_service_client(proxy_port: Option<u16>) -> Result<reqwest::Client, String> {
    workspace_access_http_client(proxy_port, Some(SERVICE_REQUEST_TIMEOUT))
        .map_err(|_| "WORKSPACE_SERVICE_CLIENT_INIT_FAILED".to_string())
}

struct ArchiveInfo {
    path: PathBuf,
    sha256: String,
    size: u64,
    uncompressed_size: u64,
    file_count: u64,
}

fn validate_archive_matches_workspace(
    path: &Path,
    workspace_metadata: &PortableWorkspaceMetadataV1,
) -> Result<ArchiveInfo, String> {
    let report = validate_workspace_archive(path);
    if !report.valid {
        return Err(report
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "ARCHIVE_INVALID".to_string()));
    }
    let archived_metadata = read_workspace_archive_metadata(path)?;
    if serde_json::to_value(&archived_metadata).ok()
        != serde_json::to_value(workspace_metadata).ok()
    {
        return Err("ARCHIVE_METADATA_DOES_NOT_MATCH_WORKSPACE".to_string());
    }
    let size = fs::metadata(path).map_err(|_| "ARCHIVE_NOT_FOUND")?.len();
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        sha256: sha256_file(path)?,
        size,
        uncompressed_size: report.total_uncompressed_size,
        file_count: report.file_count,
    })
}

fn make_public_metadata(
    request: &WorkspaceAccessPublishRequest,
    portable: PortableWorkspaceMetadataV1,
    archive: Option<&ArchiveInfo>,
) -> PublicMetadataV1 {
    let full_data = archive.map_or(
        FullData {
            available: false,
            archive_version: None,
            sha256: None,
            size: None,
            uncompressed_size: None,
            file_count: None,
        },
        |info| FullData {
            available: true,
            archive_version: Some(1),
            sha256: Some(info.sha256.clone()),
            size: Some(info.size),
            uncompressed_size: Some(info.uncompressed_size),
            file_count: Some(info.file_count),
        },
    );
    PublicMetadataV1 {
        schema_version: 1,
        game_preset: portable.game_preset,
        workspace_name: portable.workspace_name,
        description: request
            .description
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        captured_at: request.captured_at.clone(),
        game_build: request
            .game_build
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        attribution: request.attribution.clone(),
        supersedes: request
            .supersedes
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        workspace_aliases: request
            .workspace_aliases
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .take(128)
            .map(ToOwned::to_owned)
            .collect(),
        generator: Generator {
            name: "SSMT",
            version: env!("CARGO_PKG_VERSION"),
            validator_version: 1,
        },
        lods: portable.lods,
        full_data,
    }
}

async fn publish_archive<F>(
    client: &reqwest::Client,
    worker_url: &str,
    metadata: PublicMetadataV1,
    info: ArchiveInfo,
    on_progress: F,
) -> Result<WorkspacePublishResult, String>
where
    F: FnMut(WorkspaceUploadProgress),
{
    let cancellation = upload_cancellation(&info.path);
    let archive_path = info.path.clone();
    let result = publish_archive_inner(
        client,
        worker_url,
        metadata,
        info,
        on_progress,
        cancellation.clone(),
    )
    .await;
    clear_upload_cancellation(&archive_path, &cancellation);
    result
}

async fn publish_archive_inner<F>(
    client: &reqwest::Client,
    worker_url: &str,
    metadata: PublicMetadataV1,
    info: ArchiveInfo,
    mut on_progress: F,
    cancellation: Arc<AtomicBool>,
) -> Result<WorkspacePublishResult, String>
where
    F: FnMut(WorkspaceUploadProgress),
{
    if cancellation.load(Ordering::Relaxed) {
        return Err("UPLOAD_CANCELLED".to_string());
    }
    let state_path = upload_state_path(&info.path);
    let mut state = match read_state(&state_path)? {
        Some(state)
            if state.version == 2
                && state.worker_url == worker_url
                && state.archive_path == info.path.to_string_lossy()
                && state.archive_sha256 == info.sha256
                && state.archive_size == info.size
                && state.part_size == PART_SIZE =>
        {
            state
        }
        _ => {
            let response = initialize_submission(
                client,
                worker_url,
                metadata,
                info.size,
                &new_idempotency_key(),
            )
            .await?;
            // 服务端可能复用已校验的 R2 对象并直接发布，无需再请求分片。
            if response.status == "published" {
                on_progress(WorkspaceUploadProgress {
                    submission_id: response.submission_id.clone(),
                    completed_bytes: info.size,
                    total_bytes: info.size,
                    completed_parts: 0,
                    total_parts: 0,
                });
                let _ = fs::remove_file(&state_path);
                return Ok(WorkspacePublishResult {
                    submission_id: response.submission_id,
                    entry_id: response.entry_id,
                    status: response.status,
                    metadata_path: response.metadata_path,
                });
            }
            if response.status != "awaiting_upload" {
                return Err(format!(
                    "UPLOAD_INITIALIZATION_FAILED: status={}",
                    response.status
                ));
            }
            UploadState {
                version: 2,
                worker_url: worker_url.to_string(),
                archive_path: info.path.to_string_lossy().to_string(),
                archive_sha256: info.sha256.clone(),
                archive_size: info.size,
                part_size: PART_SIZE,
                submission_id: response.submission_id,
                entry_id: response.entry_id,
                part_url_expires_at: None,
                completed_parts: BTreeMap::new(),
            }
        }
    };
    let total_parts =
        u32::try_from(info.size.div_ceil(PART_SIZE)).map_err(|_| "UPLOAD_TOO_MANY_PARTS")?;
    if total_parts == 0 || total_parts > 10_000 {
        return Err("UPLOAD_TOO_MANY_PARTS".to_string());
    }
    save_state(&state_path, &state)?;

    // Initialization has succeeded and the resumable state is safely on disk.
    // Emit an initial progress event before requesting the first part URL so the
    // UI can transition out of its publish dialog as soon as uploading begins.
    let completed_bytes = state
        .completed_parts
        .keys()
        .map(|part| part_size(*part, info.size))
        .sum();
    on_progress(WorkspaceUploadProgress {
        submission_id: state.submission_id.clone(),
        completed_bytes,
        total_bytes: info.size,
        completed_parts: state.completed_parts.len() as u32,
        total_parts,
    });

    let missing: Vec<u32> = (1..=total_parts)
        .filter(|part| !state.completed_parts.contains_key(part))
        .collect();
    for numbers in missing.chunks(PART_BATCH_SIZE) {
        if cancellation.load(Ordering::Relaxed) {
            return Err("UPLOAD_CANCELLED".to_string());
        }
        let response = issue_part_urls(client, worker_url, &state.submission_id, numbers).await?;
        if response.parts.len() != numbers.len() {
            return Err("UPLOAD_PART_MISMATCH".to_string());
        }
        let returned_numbers: std::collections::BTreeSet<_> =
            response.parts.iter().map(|part| part.part_number).collect();
        let requested_numbers: std::collections::BTreeSet<_> = numbers.iter().copied().collect();
        if returned_numbers != requested_numbers {
            return Err("UPLOAD_PART_MISMATCH".to_string());
        }
        state.part_url_expires_at = response.expires_at;
        save_state(&state_path, &state)?;
        let uploads = stream::iter(response.parts.into_iter().map(|part| {
            let client = client.clone();
            let path = info.path.clone();
            let cancellation = cancellation.clone();
            async move {
                if cancellation.load(Ordering::Relaxed) {
                    return Err("UPLOAD_CANCELLED".to_string());
                }
                let part_number = part.part_number;
                let bytes =
                    tokio::task::spawn_blocking(move || read_part(&path, part_number, info.size))
                        .await
                        .map_err(|_| "ARCHIVE_READ_FAILED".to_string())??;
                let etag = upload_part_with_retry(&client, &part.url, bytes, &cancellation)
                    .await
                    .map_err(|error| format!("{error}; part={part_number}"))?;
                Ok::<(u32, String), String>((part_number, etag))
            }
        }))
        .buffer_unordered(UPLOAD_CONCURRENCY);
        futures_util::pin_mut!(uploads);
        while let Some(result) = uploads.next().await {
            let (part_number, etag) = result?;
            state.completed_parts.insert(part_number, etag);
            save_state(&state_path, &state)?;
            let completed_bytes = state
                .completed_parts
                .keys()
                .map(|part| part_size(*part, info.size))
                .sum();
            on_progress(WorkspaceUploadProgress {
                submission_id: state.submission_id.clone(),
                completed_bytes,
                total_bytes: info.size,
                completed_parts: state.completed_parts.len() as u32,
                total_parts,
            });
        }
    }
    let parts: Vec<_> = state
        .completed_parts
        .iter()
        .map(|(part_number, etag)| serde_json::json!({ "partNumber": part_number, "etag": etag }))
        .collect();
    if cancellation.load(Ordering::Relaxed) {
        return Err("UPLOAD_CANCELLED".to_string());
    }
    let response = post_json::<SubmissionResponse, _>(
        client,
        &format!(
            "{worker_url}/v1/submissions/{}/complete",
            state.submission_id
        ),
        &serde_json::json!({ "parts": parts }),
        None,
    )
    .await?;
    if response.status != "published" {
        return Err("UPLOAD_PUBLISH_INCOMPLETE".to_string());
    }
    let _ = fs::remove_file(&state_path);
    Ok(WorkspacePublishResult {
        submission_id: response.submission_id,
        entry_id: response.entry_id,
        status: response.status,
        metadata_path: response.metadata_path,
    })
}

async fn upload_part_with_retry(
    client: &reqwest::Client,
    url: &str,
    bytes: Vec<u8>,
    cancellation: &AtomicBool,
) -> Result<String, String> {
    let mut last_failure = String::new();
    for attempt in 0..UPLOAD_RETRIES {
        if cancellation.load(Ordering::Relaxed) {
            return Err("UPLOAD_CANCELLED".to_string());
        }
        let response = client.put(url).body(bytes.clone()).send().await;
        if cancellation.load(Ordering::Relaxed) {
            return Err("UPLOAD_CANCELLED".to_string());
        }
        match response {
            Ok(response) if response.status().is_success() => {
                return response
                    .headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .ok_or_else(|| "UPLOAD_ETAG_MISSING".to_string());
            }
            Ok(response) => last_failure = format!("HTTP {}", response.status().as_u16()),
            Err(error) => last_failure = transport_failure(&error).to_string(),
        }
        if attempt + 1 < UPLOAD_RETRIES {
            let jitter = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.subsec_millis() % 100)
                .unwrap_or(0);
            let delay = 250_u64.saturating_mul(1_u64 << attempt) + u64::from(jitter);
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }
    }
    Err(format!(
        "UPLOAD_PART_FAILED: {last_failure}; attempts={UPLOAD_RETRIES}"
    ))
}

fn transport_failure(error: &reqwest::Error) -> &'static str {
    // 不回显 reqwest 的 URL：分片地址携带短期签名，不能写入日志或弹窗。
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_body() {
        "body transfer failed"
    } else {
        "request failed"
    }
}

fn cancellation_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    UPLOAD_CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn upload_cancellation(path: &Path) -> Arc<AtomicBool> {
    let key = path.to_string_lossy().to_string();
    let mut registry = cancellation_registry()
        .lock()
        .expect("upload cancellation registry poisoned");
    registry
        .entry(key)
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

fn request_upload_cancellation(path: &Path) {
    upload_cancellation(path).store(true, Ordering::Relaxed);
}

fn clear_upload_cancellation(path: &Path, cancellation: &Arc<AtomicBool>) {
    let key = path.to_string_lossy().to_string();
    let mut registry = cancellation_registry()
        .lock()
        .expect("upload cancellation registry poisoned");
    if registry
        .get(&key)
        .is_some_and(|current| Arc::ptr_eq(current, cancellation))
    {
        registry.remove(&key);
    }
}

async fn initialize_submission(
    client: &reqwest::Client,
    worker_url: &str,
    metadata: PublicMetadataV1,
    archive_size: u64,
    idempotency_key: &str,
) -> Result<SubmissionResponse, String> {
    post_json(
        client,
        &format!("{worker_url}/v1/submissions"),
        &SubmissionBody {
            metadata,
            archive: ArchiveDeclaration { size: archive_size },
        },
        Some(idempotency_key),
    )
    .await
}

async fn issue_part_urls(
    client: &reqwest::Client,
    worker_url: &str,
    submission_id: &str,
    numbers: &[u32],
) -> Result<PartsResponse, String> {
    let response: PartsResponse = post_json(
        client,
        &format!("{worker_url}/v1/submissions/{submission_id}/parts"),
        &serde_json::json!({ "partNumbers": numbers }),
        None,
    )
    .await?;
    Ok(response)
}

async fn post_json<T: for<'de> Deserialize<'de>, B: Serialize>(
    client: &reqwest::Client,
    url: &str,
    body: &B,
    idempotency_key: Option<&str>,
) -> Result<T, String> {
    let mut request = client.post(url).json(body);
    if let Some(key) = idempotency_key {
        request = request.header("Idempotency-Key", key);
    }
    let response = request.send().await.map_err(|error| {
        format!(
            "WORKSPACE_SERVICE_UNAVAILABLE: {}",
            transport_failure(&error)
        )
    })?;
    let status = response.status();
    let payload = response
        .text()
        .await
        .map_err(|_| "WORKSPACE_SERVICE_INVALID_RESPONSE")?;
    if !status.is_success() {
        let error = serde_json::from_str::<ErrorResponse>(&payload).ok();
        let code = error
            .as_ref()
            .map(|error| error.error.code.as_str())
            .filter(|code| is_service_error_code(code))
            .unwrap_or("WORKSPACE_SERVICE_REQUEST_FAILED");
        let detail = error
            .as_ref()
            .and_then(|error| error.error.detail.as_deref())
            .filter(|detail| is_service_error_code(detail))
            .unwrap_or("");
        return Err(format!("{code}: HTTP {}; {detail}", status.as_u16())
            .trim_end_matches([';', ' '])
            .to_string());
    }
    serde_json::from_str(&payload).map_err(|error| {
        format!(
            "WORKSPACE_SERVICE_INVALID_RESPONSE: HTTP {}; {:?} at {}:{}",
            status.as_u16(),
            error.classify(),
            error.line(),
            error.column()
        )
    })
}

fn is_service_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == b'_')
}

fn read_part(path: &Path, part_number: u32, total_size: u64) -> Result<Vec<u8>, String> {
    let offset = (u64::from(part_number) - 1) * PART_SIZE;
    if offset >= total_size {
        return Err("UPLOAD_PART_MISMATCH".to_string());
    }
    let size = part_size(part_number, total_size);
    let mut file = File::open(path).map_err(|_| "ARCHIVE_NOT_FOUND")?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "ARCHIVE_READ_FAILED")?;
    let mut bytes = vec![0_u8; usize::try_from(size).map_err(|_| "UPLOAD_PART_TOO_LARGE")?];
    file.read_exact(&mut bytes)
        .map_err(|_| "ARCHIVE_READ_FAILED")?;
    Ok(bytes)
}

fn part_size(part_number: u32, total_size: u64) -> u64 {
    (total_size - (u64::from(part_number) - 1) * PART_SIZE).min(PART_SIZE)
}
fn upload_state_path(archive: &Path) -> PathBuf {
    let mut digest = Sha256::new();
    digest.update(archive.to_string_lossy().as_bytes());
    std::env::temp_dir()
        .join("SSMT")
        .join("WorkspaceUploads")
        .join(format!("{:x}", digest.finalize()))
        .join("state.json")
}

pub fn remove_upload_state(archive: &Path) {
    let _ = fs::remove_file(upload_state_path(archive));
}
fn normalize_worker_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(value).map_err(|_| "WORKSPACE_SERVICE_URL_INVALID")?;
    let local_host = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if parsed.scheme() == "https" || (parsed.scheme() == "http" && local_host) {
        Ok(value.to_string())
    } else {
        Err("WORKSPACE_SERVICE_URL_INVALID".to_string())
    }
}
fn read_state(path: &Path) -> Result<Option<UploadState>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "UPLOAD_STATE_INVALID".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("UPLOAD_STATE_READ_FAILED".to_string()),
    }
}
fn save_state(path: &Path, state: &UploadState) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(|_| "UPLOAD_STATE_WRITE_FAILED")?;
    let parent = path
        .parent()
        .ok_or_else(|| "UPLOAD_STATE_WRITE_FAILED".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "UPLOAD_STATE_WRITE_FAILED")?;
    let temp = parent.join("state.json.tmp");
    fs::write(&temp, bytes).map_err(|_| "UPLOAD_STATE_WRITE_FAILED")?;
    fs::rename(temp, path).map_err(|_| "UPLOAD_STATE_WRITE_FAILED".to_string())
}
fn new_idempotency_key() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let mut digest = Sha256::new();
    digest.update(format!(
        "{}:{}:{:?}",
        std::process::id(),
        timestamp,
        std::thread::current().id()
    ));
    format!("ssmt-{:x}", digest.finalize())
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod protocol_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allows_https_or_local_development_workers() {
        assert_eq!(
            normalize_worker_url("https://workspace.example/"),
            Ok("https://workspace.example".to_string())
        );
        assert_eq!(
            normalize_worker_url("http://localhost:8787/"),
            Ok("http://localhost:8787".to_string())
        );
        assert!(normalize_worker_url("http://workspace.example").is_err());
        assert!(normalize_worker_url("http://localhost.evil").is_err());
        assert!(normalize_worker_url("file:///workspace").is_err());
    }

    #[test]
    fn reads_bounded_upload_parts() {
        let path = std::env::temp_dir().join(format!("ssmt-upload-part-{}", std::process::id()));
        let data = vec![7_u8; (PART_SIZE + 3) as usize];
        fs::write(&path, &data).unwrap();
        assert_eq!(
            read_part(&path, 1, data.len() as u64).unwrap().len(),
            PART_SIZE as usize
        );
        assert_eq!(
            read_part(&path, 2, data.len() as u64).unwrap(),
            vec![7_u8; 3]
        );
        assert!(read_part(&path, 3, data.len() as u64).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancellation_signal_is_shared_by_archive_path_and_cleared_after_upload() {
        let path = PathBuf::from("C:/temp/ssmt-cancel-test.ssmtws");
        let cancellation = upload_cancellation(&path);
        assert!(!cancellation.load(Ordering::Relaxed));
        request_upload_cancellation(&path);
        assert!(cancellation.load(Ordering::Relaxed));
        clear_upload_cancellation(&path, &cancellation);
        assert!(!upload_cancellation(&path).load(Ordering::Relaxed));
        clear_upload_cancellation(&path, &upload_cancellation(&path));
    }
}
