use crate::workspace_access::diagnostics::{PublishLog, WorkspacePublishError};
use std::path::PathBuf;

use crate::workspace_access::archive::{
    create_workspace_archive, validate_workspace_archive, ArchiveBuildResult,
    ArchiveValidationReport,
};
use crate::workspace_access::library::{
    download_entry, fetch_index, fetch_metadata, record_download, LibraryDownloadResult,
    LibraryIndexV1, PublicMetadataDocument,
};
use crate::workspace_access::models::{
    WorkspaceAccessArchiveRequest, WorkspaceAccessPreflightRequest, WorkspaceAccessPublishRequest,
    WorkspacePreflightReport, WorkspacePublishResult,
};
use crate::workspace_access::transfer::{
    check_disk_space, download_archive, import_archive, import_metadata_skeleton,
    temporary_archive_path, DiskSpaceReport, DownloadResult, ImportResult,
};
use crate::workspace_access::upload::{cancel_upload, publish_workspace, remove_upload_state};
use crate::workspace_access::validate::preflight_workspace;

fn publish_log(app: &tauri::AppHandle, request: &WorkspaceAccessPublishRequest) -> PublishLog {
    PublishLog::new(
        &crate::config::path_manager::PathManager::ssmt_cache_root(app).join("Logs"),
        request,
    )
}

#[tauri::command]
pub fn workspace_access_preflight(
    request: WorkspaceAccessPreflightRequest,
) -> WorkspacePreflightReport {
    preflight_workspace(&request)
}

#[tauri::command]
pub fn workspace_access_create_archive(
    request: WorkspaceAccessArchiveRequest,
) -> Result<ArchiveBuildResult, String> {
    let output_path = PathBuf::from(request.output_path);
    create_workspace_archive(
        &WorkspaceAccessPreflightRequest {
            workspace_path: request.workspace_path,
            game_preset: request.game_preset,
            workspace_name: request.workspace_name,
        },
        &output_path,
    )
}

#[tauri::command]
pub fn workspace_access_validate_archive(path: String) -> ArchiveValidationReport {
    validate_workspace_archive(&PathBuf::from(path))
}

#[tauri::command]
pub async fn workspace_access_download_archive(
    url: String,
    destination: String,
    expected_sha256: String,
    proxy_port: Option<u16>,
) -> Result<DownloadResult, String> {
    download_archive(
        &url,
        &PathBuf::from(destination),
        &expected_sha256,
        proxy_port,
    )
    .await
}

#[tauri::command]
pub fn workspace_access_check_disk_space(path: String) -> DiskSpaceReport {
    check_disk_space(&PathBuf::from(path))
}

#[tauri::command]
pub fn workspace_access_import_archive(
    archive_path: String,
    workspace_base: String,
    workspace_name: String,
    game_preset: String,
) -> Result<ImportResult, String> {
    import_archive(
        &PathBuf::from(archive_path),
        &PathBuf::from(workspace_base),
        &workspace_name,
        &game_preset,
    )
}

#[tauri::command]
pub fn workspace_access_import_metadata_skeleton(
    metadata: crate::workspace_access::models::PortableWorkspaceMetadataV1,
    workspace_base: String,
    workspace_name: String,
) -> Result<ImportResult, String> {
    import_metadata_skeleton(&metadata, &PathBuf::from(workspace_base), &workspace_name)
}

#[tauri::command]
pub async fn workspace_access_publish(
    app: tauri::AppHandle,
    request: WorkspaceAccessPublishRequest,
) -> Result<WorkspacePublishResult, WorkspacePublishError> {
    use tauri::Emitter;
    let mut log = publish_log(&app, &request);
    let result = publish_workspace(&request, |progress| {
        log.progress(&progress);
        let _ = app.emit("workspace-access-upload-progress", progress);
    })
    .await;
    log.finish(result)
}

#[tauri::command]
pub async fn workspace_access_create_and_publish(
    app: tauri::AppHandle,
    mut request: WorkspaceAccessPublishRequest,
) -> Result<WorkspacePublishResult, WorkspacePublishError> {
    let mut log = publish_log(&app, &request);
    log.stage("archive");
    let archive_path = match temporary_archive_path(&request.workspace_name) {
        Ok(path) => path,
        Err(error) => return log.finish(Err(error)),
    };
    let created = match create_workspace_archive(
        &WorkspaceAccessPreflightRequest {
            workspace_path: request.workspace_path.clone(),
            game_preset: request.game_preset.clone(),
            workspace_name: request.workspace_name.clone(),
        },
        &archive_path,
    ) {
        Ok(created) => created,
        Err(error) => {
            let _ = std::fs::remove_file(&archive_path);
            return log.finish(Err(error));
        }
    };
    log.write(&format!("打包完成：{} 字节", created.size));
    request.archive_path = Some(archive_path.to_string_lossy().to_string());
    use tauri::Emitter;
    log.stage("initialize");
    let result = publish_workspace(&request, |progress| {
        log.progress(&progress);
        let _ = app.emit("workspace-access-upload-progress", progress);
    })
    .await;
    if result.is_err() {
        if let Err(error) =
            cancel_upload(&request.worker_url, &archive_path, request.proxy_port).await
        {
            log.write(&format!("清理未发布上传失败：{error}"));
        }
    }
    remove_upload_state(&archive_path);
    let _ = std::fs::remove_file(&archive_path);
    log.finish(result)
}

#[tauri::command]
pub async fn workspace_access_cancel_upload(
    worker_url: String,
    archive_path: String,
    proxy_port: Option<u16>,
) -> Result<(), String> {
    cancel_upload(&worker_url, &PathBuf::from(archive_path), proxy_port).await
}

#[tauri::command]
pub async fn workspace_access_fetch_index(
    raw_base_url: Option<String>,
    game_preset: String,
    proxy_port: Option<u16>,
    force_refresh: Option<bool>,
) -> Result<LibraryIndexV1, String> {
    fetch_index(
        raw_base_url.as_deref(),
        &game_preset,
        proxy_port,
        force_refresh.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn workspace_access_fetch_metadata(
    raw_base_url: Option<String>,
    metadata_path: String,
    proxy_port: Option<u16>,
) -> Result<PublicMetadataDocument, String> {
    fetch_metadata(raw_base_url.as_deref(), &metadata_path, proxy_port).await
}

#[tauri::command]
pub async fn workspace_access_download_entry(
    worker_url: String,
    entry_id: String,
    destination: String,
    expected_sha256: String,
    proxy_port: Option<u16>,
) -> Result<LibraryDownloadResult, String> {
    download_entry(
        &worker_url,
        &entry_id,
        &PathBuf::from(destination),
        &expected_sha256,
        proxy_port,
    )
    .await
}

#[tauri::command]
pub async fn workspace_access_record_download(
    worker_url: String,
    entry_id: String,
    kind: String,
    proxy_port: Option<u16>,
) -> Result<(), String> {
    record_download(&worker_url, &entry_id, &kind, proxy_port).await
}

#[tauri::command]
pub async fn workspace_access_download_and_import_entry(
    worker_url: String,
    entry_id: String,
    expected_sha256: String,
    workspace_base: String,
    workspace_name: String,
    game_preset: String,
    proxy_port: Option<u16>,
) -> Result<ImportResult, String> {
    let archive_path = temporary_archive_path(&entry_id)?;
    let result = async {
        download_entry(
            &worker_url,
            &entry_id,
            &archive_path,
            &expected_sha256,
            proxy_port,
        )
        .await?;
        import_archive(
            &archive_path,
            &PathBuf::from(workspace_base),
            &workspace_name,
            &game_preset,
        )
    }
    .await;
    let _ = std::fs::remove_file(&archive_path);
    let _ = std::fs::remove_file(archive_path.with_extension("ssmtws.part"));
    result
}
