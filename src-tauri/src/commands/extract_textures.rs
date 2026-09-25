use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::extract_new::extract_services::ExtractNewService;
use crate::helper::texture_convert_helper::TextureConvertHelper;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

fn resolve_lod_workspace_path(workspace_root_path: &str, lod_name: &str) -> Result<String, String> {
    let trimmed_workspace_root_path = workspace_root_path.trim();
    if trimmed_workspace_root_path.is_empty() {
        return Err("workspace_root_path is empty".to_string());
    }

    let trimmed_lod_name = lod_name.trim();
    if trimmed_lod_name.is_empty() {
        return Err("lod_name is empty".to_string());
    }

    Ok(PathBuf::from(trimmed_workspace_root_path)
        .join(trimmed_lod_name)
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub async fn extract_deduped_textures(
    _app: AppHandle,
    frame_analysis_folder: String,
    game_preset: String,
    workspace_root_path: String,
    lod_name: String,
) -> Result<(), String> {
    let fa = FrameAnalysis::new(&frame_analysis_folder)
        .map_err(|e| format!("Failed to read FrameAnalysis data: {}", e))?;

    let lod_workspace_path = resolve_lod_workspace_path(&workspace_root_path, &lod_name)?;

    ExtractNewService::extract_deduped_textures(
        &fa,
        &game_preset,
        &workspace_root_path,
        &lod_workspace_path,
    )?;

    Ok(())
}

#[tauri::command]
pub async fn extract_trianglelist_textures(
    _app: AppHandle,
    frame_analysis_folder: String,
    game_preset: String,
    workspace_root_path: String,
    lod_name: String,
) -> Result<(), String> {
    let fa = FrameAnalysis::new(&frame_analysis_folder)
        .map_err(|e| format!("Failed to read FrameAnalysis data: {}", e))?;

    let lod_workspace_path = resolve_lod_workspace_path(&workspace_root_path, &lod_name)?;

    ExtractNewService::extract_trianglelist_textures(
        &fa,
        &game_preset,
        &workspace_root_path,
        &lod_workspace_path,
    )?;

    Ok(())
}

#[tauri::command]
pub async fn prepare_dds_webgl_preview(
    app: AppHandle,
    source_path: String,
    max_dimension: Option<u32>,
) -> Result<String, String> {
    let source = Path::new(source_path.trim());
    if !source.is_file()
        || !source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dds"))
    {
        return Err("DDS preview source does not exist or is not a DDS file".to_string());
    }

    let metadata = std::fs::metadata(source).map_err(|error| error.to_string())?;
    let mut hasher = DefaultHasher::new();
    // Bump when conversion semantics change so an older artifact can never be
    // reused merely because its source identity still matches.
    "dds-webgl-preview-v2".hash(&mut hasher);
    source.to_string_lossy().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified().ok().hash(&mut hasher);
    let max_dimension = max_dimension.filter(|value| *value > 0);
    max_dimension.hash(&mut hasher);
    let cache_key = format!("{:016x}", hasher.finish());
    let cache_root = crate::config::path_manager::PathManager::ssmt_cache_root(&app);
    let cache_dir = cache_root.join("DdsWebglPreview").join(&cache_key);
    std::fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;
    let output = cache_dir.join(
        source
            .file_name()
            .ok_or_else(|| "DDS source has no file name".to_string())?,
    );
    static CONVERSION_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let conversion_lock = {
        let locks = CONVERSION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = locks.lock().map_err(|_| "DDS preview lock is poisoned")?;
        locks.entry(cache_key.clone()).or_default().clone()
    };
    let _conversion_guard = conversion_lock
        .lock()
        .map_err(|_| "DDS conversion lock is poisoned")?;
    if !output.is_file() {
        if let Some(limit) = max_dimension {
            let mut header = [0u8; 20];
            std::fs::File::open(source)
                .and_then(|mut file| file.read_exact(&mut header))
                .map_err(|error| error.to_string())?;
            if &header[0..4] != b"DDS " {
                return Err("Invalid DDS header".to_string());
            }
            let height = u32::from_le_bytes(header[12..16].try_into().unwrap());
            let width = u32::from_le_bytes(header[16..20].try_into().unwrap());
            let scale = (limit as f64 / width.max(height).max(1) as f64).min(1.0);
            let target_width = ((width as f64 * scale).round() as u32).max(1);
            let target_height = ((height as f64 * scale).round() as u32).max(1);
            TextureConvertHelper::convert_texture_to_target_fmt_with_size(
                &source.to_string_lossy(),
                &cache_dir.to_string_lossy(),
                "dds",
                target_width,
                target_height,
            )?;
        } else {
            TextureConvertHelper::convert_texture_to_rgba_dds(
                &source.to_string_lossy(),
                &cache_dir.to_string_lossy(),
            )?;
        }
    }
    if !output.is_file() {
        return Err("DirectXTex did not create the DDS preview cache".to_string());
    }
    Ok(output.to_string_lossy().to_string())
}
