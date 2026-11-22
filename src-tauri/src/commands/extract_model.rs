use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::constants::gametype::ExtractTechnique;
use crate::extract_new::extract_services::{ExtractNewService, FullExtractDataTypeFilter};
use crate::helper::mark_texture_helper::MarkTextureHelper;

#[derive(Clone, Copy)]
enum ExtractionLogLanguage {
    Chinese,
    English,
}

impl ExtractionLogLanguage {
    fn from_input(value: Option<&str>) -> Self {
        if value == Some("en") {
            Self::English
        } else {
            Self::Chinese
        }
    }
}

struct ExtractionLogContext<'a> {
    kind: &'a str,
    frame_analysis_folder: &'a str,
    game_preset: &'a str,
    workspace_root_path: &'a str,
    lod_name: &'a str,
    data_type_filter: Option<&'a str>,
}

fn extraction_output_stats(lod_workspace_path: &str) -> (usize, usize, Vec<String>) {
    fn visit(path: &std::path::Path, files: &mut usize, directories: &mut usize) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                *directories += 1;
                visit(&child, files, directories);
            } else if child.is_file() {
                *files += 1;
            }
        }
    }

    let root = std::path::Path::new(lod_workspace_path);
    let mut files = 0;
    let mut directories = 0;
    visit(root, &mut files, &mut directories);
    let mut top_level = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    top_level.sort();
    (files, directories, top_level)
}

fn write_extraction_log(
    requested_folder: Option<&str>,
    language: Option<&str>,
    context: &ExtractionLogContext<'_>,
    lod_workspace_path: Option<&str>,
    started_at: chrono::DateTime<chrono::Local>,
    elapsed: std::time::Duration,
    result: &Result<(), String>,
    diagnostic_lines: Vec<String>,
) {
    let lang = ExtractionLogLanguage::from_input(language);
    let fallback = PathBuf::from(context.workspace_root_path).join("Logs");
    let folder = requested_folder
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(fallback);
    if let Err(error) = fs::create_dir_all(&folder) {
        eprintln!(
            "Failed to create extraction log folder {}: {}",
            folder.display(),
            error
        );
        return;
    }

    let stamp = started_at.format("%Y%m%dT%H%M%S%.3f").to_string();
    let path = folder.join(format!("{}-model-extract.log", stamp));
    let timestamp =
        |time: chrono::DateTime<chrono::Local>| time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let end_time = chrono::Local::now();
    let (files, directories, top_level) = lod_workspace_path
        .map(extraction_output_stats)
        .unwrap_or_default();

    let mut lines = match lang {
        ExtractionLogLanguage::Chinese => vec![
            format!("{} [INFO] SSMT4 模型提取日志", timestamp(started_at)),
            format!(
                "{} [INFO] 提取类型: {}",
                timestamp(started_at),
                if context.kind == "full" {
                    "完整提取"
                } else {
                    "按 DrawIB 提取"
                }
            ),
            format!(
                "{} [INFO] 游戏预设: {}",
                timestamp(started_at),
                context.game_preset
            ),
            format!(
                "{} [INFO] FrameAnalysis 文件夹: {}",
                timestamp(started_at),
                context.frame_analysis_folder
            ),
            format!(
                "{} [INFO] 工作空间根目录: {}",
                timestamp(started_at),
                context.workspace_root_path
            ),
            format!("{} [INFO] LOD: {}", timestamp(started_at), context.lod_name),
            format!(
                "{} [INFO] 数据类型筛选: {}",
                timestamp(started_at),
                context.data_type_filter.unwrap_or("all")
            ),
            format!(
                "{} [INFO] 输出文件: {}，输出目录: {}",
                timestamp(end_time),
                files,
                directories
            ),
            format!(
                "{} [INFO] 顶层输出目录: {}",
                timestamp(end_time),
                if top_level.is_empty() {
                    "（无）".to_string()
                } else {
                    top_level.join(", ")
                }
            ),
            match result {
                Ok(()) => format!("{} [INFO] 提取成功", timestamp(end_time)),
                Err(error) => format!("{} [ERROR] 提取失败: {}", timestamp(end_time), error),
            },
            format!(
                "{} [INFO] 本次提取总耗时: {:.3} 秒",
                timestamp(end_time),
                elapsed.as_secs_f64()
            ),
        ],
        ExtractionLogLanguage::English => vec![
            format!(
                "{} [INFO] SSMT4 model extraction log",
                timestamp(started_at)
            ),
            format!(
                "{} [INFO] Extraction type: {}",
                timestamp(started_at),
                if context.kind == "full" {
                    "full extraction"
                } else {
                    "DrawIB extraction"
                }
            ),
            format!(
                "{} [INFO] Game preset: {}",
                timestamp(started_at),
                context.game_preset
            ),
            format!(
                "{} [INFO] FrameAnalysis folder: {}",
                timestamp(started_at),
                context.frame_analysis_folder
            ),
            format!(
                "{} [INFO] Workspace root: {}",
                timestamp(started_at),
                context.workspace_root_path
            ),
            format!("{} [INFO] LOD: {}", timestamp(started_at), context.lod_name),
            format!(
                "{} [INFO] Data-type filter: {}",
                timestamp(started_at),
                context.data_type_filter.unwrap_or("all")
            ),
            format!(
                "{} [INFO] Output files: {}; output directories: {}",
                timestamp(end_time),
                files,
                directories
            ),
            format!(
                "{} [INFO] Top-level output directories: {}",
                timestamp(end_time),
                if top_level.is_empty() {
                    "(none)".to_string()
                } else {
                    top_level.join(", ")
                }
            ),
            match result {
                Ok(()) => format!(
                    "{} [INFO] Extraction completed successfully",
                    timestamp(end_time)
                ),
                Err(error) => format!(
                    "{} [ERROR] Extraction failed: {}",
                    timestamp(end_time),
                    error
                ),
            },
            format!(
                "{} [INFO] Total extraction time: {:.3} seconds",
                timestamp(end_time),
                elapsed.as_secs_f64()
            ),
        ],
    };

    let summary_start = lines.len().saturating_sub(4);
    let detail_heading = match lang {
        ExtractionLogLanguage::Chinese => format!(
            "{} [INFO] -------------------- 提取诊断明细 --------------------",
            timestamp(started_at)
        ),
        ExtractionLogLanguage::English => format!(
            "{} [INFO] ---------------- Extraction diagnostics ----------------",
            timestamp(started_at)
        ),
    };
    lines.insert(summary_start, detail_heading);
    lines.splice(summary_start + 1..summary_start + 1, diagnostic_lines);

    if let Err(error) = fs::write(&path, lines.join("\r\n") + "\r\n") {
        eprintln!(
            "Failed to write extraction log {}: {}",
            path.display(),
            error
        );
    }
}

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

/// Rebuild the derived component map from the extracted folders already on disk.
/// This deliberately does not consult Import.json: that file is Blender's
/// selected-data-type state, not the set of extracted/importable components.
#[tauri::command]
pub fn regenerate_draw_ib_component_json(lod_workspace_path: String) -> Result<(), String> {
    let lod_workspace_path = lod_workspace_path.trim();
    if lod_workspace_path.is_empty() {
        return Err("lod_workspace_path is empty".to_string());
    }

    let lod_path = PathBuf::from(lod_workspace_path);
    if !lod_path.is_dir() {
        return Err(format!(
            "LOD workspace path does not exist: {}",
            lod_workspace_path
        ));
    }

    let repaired_count =
        MarkTextureHelper::reset_gimi_vertex_ranges_for_import(lod_workspace_path)?;
    if repaired_count > 0 {
        println!(
            "Restored {} GIMI submesh JSON files to full-buffer import semantics",
            repaired_count
        );
    }
    MarkTextureHelper::generate_draw_ib_component_json(lod_workspace_path);
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DrawIBSubmeshRange {
    #[serde(rename = "firstIndex")]
    pub first_index: String,
    #[serde(rename = "indexCount")]
    pub index_count: String,
}

#[tauri::command]
pub async fn extract_models_new(
    frame_analysis_folder: String,
    game_preset: String,
    workspace_root_path: String,
    lod_name: String,
    log_folder: Option<String>,
    log_language: Option<String>,
) -> Result<(), String> {
    let started_at = chrono::Local::now();
    let timer = Instant::now();
    crate::utils::extraction_log::begin(log_language.as_deref());
    let lod_workspace_path = resolve_lod_workspace_path(&workspace_root_path, &lod_name);
    let result = lod_workspace_path
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|path| {
            if game_preset == "ZZMIDX12" {
                return ExtractNewService::run_extract_dx12(
                    &frame_analysis_folder,
                    path,
                    FullExtractDataTypeFilter::All,
                    false,
                );
            }
            let fa = FrameAnalysis::new(&frame_analysis_folder)
                .map_err(|e| format!("Failed to read FrameAnalysis data: {}", e))?;
            ExtractNewService::run_extract(
                &fa,
                &game_preset,
                &workspace_root_path,
                path,
                FullExtractDataTypeFilter::All,
                false,
            )
        });
    let context = ExtractionLogContext {
        kind: "drawib",
        frame_analysis_folder: &frame_analysis_folder,
        game_preset: &game_preset,
        workspace_root_path: &workspace_root_path,
        lod_name: &lod_name,
        data_type_filter: None,
    };
    write_extraction_log(
        log_folder.as_deref(),
        log_language.as_deref(),
        &context,
        lod_workspace_path.as_deref().ok(),
        started_at,
        timer.elapsed(),
        &result,
        crate::utils::extraction_log::take(),
    );
    result
}

#[tauri::command]
pub async fn full_extract(
    frame_analysis_folder: String,
    game_preset: String,
    workspace_root_path: String,
    lod_name: String,
    data_type_filter: String,
    log_folder: Option<String>,
    log_language: Option<String>,
) -> Result<(), String> {
    let start_time = Instant::now();
    let started_at = chrono::Local::now();
    crate::utils::extraction_log::begin(log_language.as_deref());
    let filter_result = FullExtractDataTypeFilter::from_input(&data_type_filter);
    let lod_workspace_path = resolve_lod_workspace_path(&workspace_root_path, &lod_name);
    let result = filter_result.and_then(|filter| {
        lod_workspace_path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|path| {
                if game_preset == "ZZMIDX12" {
                    return ExtractNewService::run_extract_dx12(
                        &frame_analysis_folder,
                        path,
                        filter,
                        true,
                    );
                }
                let fa = FrameAnalysis::new(&frame_analysis_folder)
                    .map_err(|e| format!("Failed to read FrameAnalysis data: {}", e))?;
                ExtractNewService::run_extract(
                    &fa,
                    &game_preset,
                    &workspace_root_path,
                    path,
                    filter,
                    true,
                )
            })
    });
    let elapsed = start_time.elapsed();
    crate::extract_log!(
        "[full_extract] Total execution time: {:.3}s ({} ms)",
        elapsed.as_secs_f64(),
        elapsed.as_millis()
    );

    let context = ExtractionLogContext {
        kind: "full",
        frame_analysis_folder: &frame_analysis_folder,
        game_preset: &game_preset,
        workspace_root_path: &workspace_root_path,
        lod_name: &lod_name,
        data_type_filter: Some(&data_type_filter),
    };
    write_extraction_log(
        log_folder.as_deref(),
        log_language.as_deref(),
        &context,
        lod_workspace_path.as_deref().ok(),
        started_at,
        elapsed,
        &result,
        crate::utils::extraction_log::take(),
    );
    result
}

#[tauri::command]
pub async fn analyze_draw_ib_submeshes(
    frame_analysis_folder: String,
    draw_ib: String,
) -> Result<Vec<DrawIBSubmeshRange>, String> {
    let draw_ib = draw_ib.trim();
    if draw_ib.is_empty() {
        return Ok(Vec::new());
    }

    let fa = FrameAnalysis::new(&frame_analysis_folder)
        .map_err(|e| format!("Failed to read FrameAnalysis data: {}", e))?;

    let ib_txt_files = fa.data.filter_filelist(&format!("-ib={}", draw_ib), ".txt");

    let mut submesh_set: BTreeSet<(u64, u64)> = BTreeSet::new();

    for ib_txt_file_name in ib_txt_files {
        let ib_txt_path = fa.data.dir.join(&ib_txt_file_name);
        let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_path, false)?;

        if ib_txt_file.topology != ExtractTechnique::TRIANGLELIST {
            continue;
        }

        let first_index = ib_txt_file.first_index.trim().parse::<u64>().unwrap_or(0);
        let raw_index_count = ib_txt_file.index_count.trim().parse::<u64>().unwrap_or(0);
        let index_count = if raw_index_count == 0 {
            ib_txt_file.index_number_count
        } else {
            raw_index_count
        };

        if index_count == 0 {
            continue;
        }

        submesh_set.insert((first_index, index_count));
    }

    Ok(submesh_set
        .into_iter()
        .map(|(first_index, index_count)| DrawIBSubmeshRange {
            first_index: first_index.to_string(),
            index_count: index_count.to_string(),
        })
        .collect())
}
