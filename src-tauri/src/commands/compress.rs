use std::path::PathBuf;

use crate::utils::ssmt_compress_utils::SSMTCompressUtils;

#[tauri::command]
pub async fn extract_zip_archive(zip_path: String, dest_dir: String) -> Result<(), String> {
    let zip_path = PathBuf::from(&zip_path);
    let dest_dir = PathBuf::from(&dest_dir);

    if !zip_path.exists() {
        return Err(format!(
            "Zip file not found: {}",
            zip_path.to_string_lossy()
        ));
    }

    if !dest_dir.exists() {
        std::fs::create_dir_all(&dest_dir)
            .map_err(|e| format!("Failed to create destination: {}", e))?;
    }

    SSMTCompressUtils::extract_zip_archive(&zip_path, &dest_dir, |_current, _total| {}).map(|_| ())
}

/// 通用解压命令:按扩展名自动选择 zip / 7z / rar 解压器,把压缩包解压到
/// 目标目录(自动创建)。芝士猫「拖入压缩包作为工作目录」等功能使用。
#[tauri::command]
pub async fn extract_archive_to_dir(archive_path: String, dest_dir: String) -> Result<(), String> {
    let archive_path = PathBuf::from(&archive_path);
    let dest_dir = PathBuf::from(&dest_dir);

    if !archive_path.exists() {
        return Err(format!(
            "Archive file not found: {}",
            archive_path.to_string_lossy()
        ));
    }

    if !dest_dir.exists() {
        std::fs::create_dir_all(&dest_dir)
            .map_err(|e| format!("Failed to create destination: {}", e))?;
    }

    let lower_name = archive_path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let extraction = tokio::task::spawn_blocking(move || {
        if lower_name.ends_with(".zip") {
            SSMTCompressUtils::extract_zip_archive(&archive_path, &dest_dir, |_current, _total| {})
        } else if lower_name.ends_with(".7z") {
            SSMTCompressUtils::extract_7z_archive(&archive_path, &dest_dir, |_current, _total| {})
        } else if lower_name.ends_with(".rar") {
            SSMTCompressUtils::extract_rar_archive(&archive_path, &dest_dir, |_current, _total| {})
        } else {
            Err(format!("Unsupported archive format: {}", lower_name))
        }
    })
    .await
    .map_err(|e| format!("Extraction task failed: {e}"))??;

    if extraction.processed == 0 {
        return Err("Archive contained no extractable files.".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn create_rar_archive(source_dir: String, output_path: String) -> Result<(), String> {
    let source_dir = PathBuf::from(&source_dir);
    let output_path = PathBuf::from(&output_path);

    SSMTCompressUtils::create_rar_archive(&source_dir, &output_path)
}

#[tauri::command]
pub async fn create_mod_archive(
    source_dir: String,
    output_path: String,
    format: String,
    password: Option<String>,
) -> Result<(), String> {
    let source_dir = PathBuf::from(&source_dir);
    let output_path = PathBuf::from(&output_path);

    SSMTCompressUtils::create_mod_archive(&source_dir, &output_path, &format, password.as_deref())
}
