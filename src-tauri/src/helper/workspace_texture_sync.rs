use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::frame_analysis::frameanalysis_data::FrameAnalysisData;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::config::drawib_config::DrawIBConfig;
use crate::helper::mark_texture_helper::{
    get_workspace_component_name_draw_call_index_list_json_path,
    get_workspace_trianglelist_deduped_filename_json_path, ComponentNameDrawCallIndexListJson,
    TrianglelistDedupedFileNameJson, TrianglelistDedupedTextureProperty,
};
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::utils::ssmt_string_utils::SSMTStringUtils;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

struct YyslsCtxInfo {
    trianglelist_index_list: Vec<String>,
    ctx_data: FrameAnalysisData,
    filename_deduped_map: HashMap<String, String>,
}

fn push_unique_string(target: &mut Vec<String>, value: &str) {
    if value.is_empty() || target.iter().any(|item| item == value) {
        return;
    }

    target.push(value.to_string());
}

fn resolve_trianglelist_index_list_for_draw_ib(fa: &FrameAnalysis, draw_ib: &str) -> Vec<String> {
    let mut trianglelist_index_list = fa.data.get_trianglelist_index_list(draw_ib);
    if !trianglelist_index_list.is_empty() {
        return trianglelist_index_list;
    }

    let trianglelist_ib_file_list = fa
        .data
        .filter_filelist_by_content_and_suffix(&format!("-ib={}", draw_ib), ".txt");

    for trianglelist_ib_file_name in trianglelist_ib_file_list.iter() {
        let index = trianglelist_ib_file_name
            .get(0..6)
            .unwrap_or_default()
            .trim()
            .to_string();
        if index.is_empty() {
            continue;
        }

        if !trianglelist_index_list.contains(&index) {
            trianglelist_index_list.push(index);
        }
    }

    trianglelist_index_list
}

fn parse_filename_deduped_map_from_log_file(
    log_txt_file_path: &Path,
) -> Result<HashMap<String, String>, String> {
    let raw = fs::read(log_txt_file_path).map_err(|e| {
        format!(
            "Failed to read ctx log file {}: {}",
            log_txt_file_path.to_string_lossy(),
            e
        )
    })?;
    let content = String::from_utf8_lossy(&raw).into_owned();

    let mut out = HashMap::new();
    let dumping_texture_2d = "Dumping Texture2D";
    let dumping_buffer = "Dumping Buffer";

    for log_line in content.lines() {
        if !log_line.contains("->") {
            continue;
        }

        let start = if let Some(idx) = log_line.find(dumping_texture_2d) {
            idx + dumping_texture_2d.len()
        } else if let Some(idx) = log_line.find(dumping_buffer) {
            idx + dumping_buffer.len()
        } else {
            continue;
        };

        let path_splits: Vec<&str> = log_line[start..].split("->").collect();
        if path_splits.len() < 2 {
            continue;
        }

        let original_raw = path_splits[path_splits.len() - 2].trim();
        let deduped_raw = path_splits[path_splits.len() - 1].trim();

        let original_file_name = Path::new(original_raw)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(original_raw)
            .to_string();
        let deduped_file_name = Path::new(deduped_raw)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(deduped_raw)
            .to_string();

        out.insert(original_file_name, deduped_file_name);
    }

    Ok(out)
}

fn resolve_yysls_ctx_info_list_for_draw_ib(
    frame_analysis_folder: &Path,
    draw_ib: &str,
) -> Result<Vec<YyslsCtxInfo>, String> {
    let mut log_txt_file_path_list: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(frame_analysis_folder).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };

        if file_name.starts_with("log-") && file_name.ends_with(".txt") {
            log_txt_file_path_list.push(path);
        }
    }
    log_txt_file_path_list.sort();

    let mut out: Vec<YyslsCtxInfo> = Vec::new();
    for log_txt_file_path in log_txt_file_path_list {
        let raw = fs::read(&log_txt_file_path).map_err(|e| {
            format!(
                "Failed to read log file {}: {}",
                log_txt_file_path.to_string_lossy(),
                e
            )
        })?;
        let content = String::from_utf8_lossy(&raw).into_owned();
        if !content
            .lines()
            .any(|line| line.contains(&format!("hash={}", draw_ib)))
        {
            continue;
        }

        let log_file_name = log_txt_file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let ctx_code = log_file_name
            .strip_prefix("log-")
            .unwrap_or_default()
            .strip_suffix(".txt")
            .unwrap_or_default()
            .to_string();
        if ctx_code.is_empty() {
            continue;
        }

        let ctx_folder_path = frame_analysis_folder.join(format!("ctx-{}", ctx_code));
        if !ctx_folder_path.exists() || !ctx_folder_path.is_dir() {
            continue;
        }

        let ctx_data = FrameAnalysisData::new(&ctx_folder_path.to_string_lossy())?;
        let trianglelist_ib_file_list =
            ctx_data.filter_filelist(&format!("-ib={}", draw_ib), ".txt");
        let mut trianglelist_index_list: Vec<String> = Vec::new();
        for trianglelist_ib_file_name in trianglelist_ib_file_list {
            let index = trianglelist_ib_file_name
                .get(0..6)
                .unwrap_or_default()
                .trim()
                .to_string();
            if index.is_empty() {
                continue;
            }
            if !trianglelist_index_list.contains(&index) {
                trianglelist_index_list.push(index);
            }
        }
        if trianglelist_index_list.is_empty() {
            continue;
        }

        let filename_deduped_map = parse_filename_deduped_map_from_log_file(&log_txt_file_path)?;
        out.push(YyslsCtxInfo {
            trianglelist_index_list,
            ctx_data,
            filename_deduped_map,
        });
    }

    Ok(out)
}

fn find_deduped_filename_in_root_by_hash(frame_analysis_folder: &Path, hash: &str) -> String {
    let deduped_dir = frame_analysis_folder.join("deduped");
    if !deduped_dir.exists() {
        return String::new();
    }

    if let Ok(entries) = fs::read_dir(&deduped_dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            if let Some(file_name_str) = file_name.to_str() {
                if file_name_str.contains(hash) {
                    return format!("{}_{}", hash, file_name_str);
                }
            }
        }
    }

    String::new()
}

pub fn sync_yysls_workspace_deduped_textures_and_json(
    fa: &FrameAnalysis,
    drawib_config: &DrawIBConfig,
    workspace_path: &str,
) -> Result<(), String> {
    let frame_analysis_folder = PathBuf::from(&fa.folder_path);

    let mut component_drawcall_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();
    let mut trianglelist_deduped_map: HashMap<String, TrianglelistDedupedTextureProperty> =
        HashMap::new();

    let deduped_folder_path = PathBuf::from(workspace_path).join("DedupedTextures");
    SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path)?;

    for entry in drawib_config.entries.iter() {
        let draw_ib = entry.draw_ib.trim();
        if draw_ib.is_empty() {
            continue;
        }

        let ctx_info_list =
            resolve_yysls_ctx_info_list_for_draw_ib(&frame_analysis_folder, draw_ib)?;
        if ctx_info_list.is_empty() {
            continue;
        }

        let mut draw_ib_trianglelist_index_list: Vec<String> = Vec::new();
        let mut first_index_index_count_map: BTreeMap<u64, String> = BTreeMap::new();
        let mut first_index_trianglelist_index_map: BTreeMap<u64, Vec<String>> = BTreeMap::new();

        for ctx_info in &ctx_info_list {
            for trianglelist_index in &ctx_info.trianglelist_index_list {
                push_unique_string(&mut draw_ib_trianglelist_index_list, trianglelist_index);

                let search_key = format!("{}-ib", trianglelist_index);
                let ib_txt_filename = ctx_info
                    .ctx_data
                    .filter_first_file(&search_key, ".txt")
                    .unwrap_or_default();
                if ib_txt_filename.is_empty() {
                    continue;
                }
                let Some(deduped_ib_file_name) =
                    ctx_info.filename_deduped_map.get(&ib_txt_filename)
                else {
                    continue;
                };
                let ib_txt_filepath = frame_analysis_folder
                    .join("deduped")
                    .join(deduped_ib_file_name);
                if !ib_txt_filepath.exists() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_filepath, true)?;
                let first_index = ib_txt_file.first_index.trim().parse::<u64>().unwrap_or(0);
                let index_count = ib_txt_file.index_number_count.to_string();
                if !index_count.is_empty() {
                    first_index_index_count_map
                        .entry(first_index)
                        .or_insert(index_count);
                }
                push_unique_string(
                    first_index_trianglelist_index_map
                        .entry(first_index)
                        .or_default(),
                    trianglelist_index,
                );
            }

            for trianglelist_index in &ctx_info.trianglelist_index_list {
                let content_str = format!("{}-ps-t", trianglelist_index);
                let ps_texture_all_filename_list =
                    ctx_info.ctx_data.filter_texture_filename_list(&content_str);

                for ps_texture_filename in ps_texture_all_filename_list {
                    let hash = SSMTStringUtils::get_file_hash_from_file_name(&ps_texture_filename);
                    let fa_data_deduped_filename =
                        find_deduped_filename_in_root_by_hash(&frame_analysis_folder, &hash);

                    let fa_log_deduped_filename = ctx_info
                        .filename_deduped_map
                        .get(&ps_texture_filename)
                        .map(|deduped| format!("{}_{}", hash, deduped))
                        .unwrap_or_default();

                    if !fa_log_deduped_filename.is_empty() {
                        let deduped_source_path =
                            deduped_folder_path_of_frame_analysis(&frame_analysis_folder).join(
                                fa_log_deduped_filename
                                    .split_once('_')
                                    .map(|(_, rest)| rest)
                                    .unwrap_or_default(),
                            );
                        if deduped_source_path.exists() {
                            let target_texture_path =
                                deduped_folder_path.join(&fa_log_deduped_filename);
                            let target_texture_path_str =
                                target_texture_path.to_string_lossy().to_string();
                            let source_path_str = deduped_source_path.to_string_lossy().to_string();
                            let _ = SSMTFileUtils::copy_to_file_if_not_exists(
                                &source_path_str,
                                &target_texture_path_str,
                            );
                        }
                    }

                    trianglelist_deduped_map
                        .entry(ps_texture_filename)
                        .or_insert_with(|| TrianglelistDedupedTextureProperty {
                            fa_log_deduped_file_name: fa_log_deduped_filename.clone(),
                            fa_data_deduped_file_name: fa_data_deduped_filename,
                            format: SSMTStringUtils::extract_texture_format_from_deduped_file_name(
                                &fa_log_deduped_filename,
                            ),
                        });
                }
            }
        }

        if first_index_trianglelist_index_map.is_empty() {
            component_drawcall_index_list_dict
                .insert(draw_ib.to_string(), draw_ib_trianglelist_index_list);
        } else {
            for (first_index, trianglelist_index_list) in first_index_trianglelist_index_map {
                let index_count = first_index_index_count_map
                    .get(&first_index)
                    .cloned()
                    .unwrap_or_default();
                let submesh_folder_name = format!("{}-{}-{}", draw_ib, index_count, first_index);
                component_drawcall_index_list_dict
                    .insert(submesh_folder_name, trianglelist_index_list);
            }
        }
    }

    if component_drawcall_index_list_dict.is_empty() {
        return Ok(());
    }

    let component_json_path =
        get_workspace_component_name_draw_call_index_list_json_path(workspace_path)?;
    let component_json =
        ComponentNameDrawCallIndexListJson::from_map(component_drawcall_index_list_dict);
    if let Err(e) = component_json.save_to_file(&component_json_path) {
        crate::extract_error!(
            "Failed to write {}: {}",
            component_json_path.to_string_lossy(),
            e
        );
    }

    let trianglelist_json_path =
        get_workspace_trianglelist_deduped_filename_json_path(workspace_path)?;
    let trianglelist_json = TrianglelistDedupedFileNameJson::from_map(trianglelist_deduped_map);
    if let Err(e) = trianglelist_json.save_to_file(&trianglelist_json_path) {
        crate::extract_error!(
            "Failed to write {}: {}",
            trianglelist_json_path.to_string_lossy(),
            e
        );
    }

    Ok(())
}

fn deduped_folder_path_of_frame_analysis(frame_analysis_folder: &Path) -> PathBuf {
    frame_analysis_folder.join("deduped")
}

pub fn sync_workspace_deduped_textures_and_json(
    fa: &FrameAnalysis,
    drawib_config: &DrawIBConfig,
    workspace_path: &str,
) -> Result<(), String> {
    sync_workspace_deduped_textures_and_json_impl(fa, drawib_config, workspace_path, false)
}

// CAMI（卡拉彼丘）的 FA 转储含 trianglestrip 等非 trianglelist 的 ib txt（无 first/count 语义），
// 参与分组会把组件名污染成 {draw_ib}-0-0 这类错误文件夹名，且单个坏文件不应中止整体同步。
// 仅 CAMI 启用该容错，其余预设行为保持不变。
pub fn sync_cami_workspace_deduped_textures_and_json(
    fa: &FrameAnalysis,
    drawib_config: &DrawIBConfig,
    workspace_path: &str,
) -> Result<(), String> {
    sync_workspace_deduped_textures_and_json_impl(fa, drawib_config, workspace_path, true)
}

fn sync_workspace_deduped_textures_and_json_impl(
    fa: &FrameAnalysis,
    drawib_config: &DrawIBConfig,
    workspace_path: &str,
    skip_non_trianglelist_ib: bool,
) -> Result<(), String> {
    let mut component_drawcall_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();

    for entry in drawib_config.entries.iter() {
        let draw_ib = entry.draw_ib.trim();
        if draw_ib.is_empty() {
            continue;
        }

        let trianglelist_index_list = resolve_trianglelist_index_list_for_draw_ib(fa, draw_ib);
        if trianglelist_index_list.is_empty() {
            continue;
        }

        let mut first_index_ib_txt_map: BTreeMap<u64, String> = BTreeMap::new();
        let mut first_index_index_count_map: BTreeMap<u64, String> = BTreeMap::new();
        let mut first_index_trianglelist_index_map: BTreeMap<u64, Vec<String>> = BTreeMap::new();

        for trianglelist_index in trianglelist_index_list.iter() {
            let search_key = format!("{}-ib", trianglelist_index);
            let ib_txt_filename = fa
                .data
                .filter_first_file_by_content_and_suffix(&search_key, ".txt")
                .unwrap_or_default();
            if ib_txt_filename.is_empty() {
                continue;
            }

            let ib_txt_filepath = fa.log.get_deduped_filepath(&ib_txt_filename);
            if ib_txt_filepath.is_empty() || !Path::new(&ib_txt_filepath).exists() {
                continue;
            }

            let ib_txt_file = if skip_non_trianglelist_ib {
                match IndexBufferTxtFile::new(&ib_txt_filepath, true) {
                    Ok(file) => file,
                    Err(error) => {
                        crate::extract_log!(
                            "跳过无法解析的 IB txt {}: {}",
                            ib_txt_filename, error
                        );
                        continue;
                    }
                }
            } else {
                IndexBufferTxtFile::new(&ib_txt_filepath, true)?
            };
            // 非 trianglelist（如 trianglestrip）的 ib txt 没有 first/count 语义，
            // 参与分组会把组件名污染成 {draw_ib}-0-0 这类错误文件夹名。
            if skip_non_trianglelist_ib && ib_txt_file.topology != "trianglelist" {
                crate::extract_log!(
                    "跳过非 trianglelist 的 IB txt: {} (topology={})",
                    ib_txt_filename, ib_txt_file.topology
                );
                continue;
            }
            let first_index = ib_txt_file.first_index.trim().parse::<u64>().unwrap_or(0);
            first_index_ib_txt_map.insert(first_index, ib_txt_filename);
            first_index_index_count_map
                .insert(first_index, ib_txt_file.index_number_count.to_string());
            push_unique_string(
                first_index_trianglelist_index_map
                    .entry(first_index)
                    .or_default(),
                trianglelist_index,
            );
        }

        if first_index_ib_txt_map.is_empty() {
            component_drawcall_index_list_dict
                .insert(draw_ib.to_string(), trianglelist_index_list.clone());
            continue;
        }

        for (first_index, submesh_trianglelist_index_list) in first_index_trianglelist_index_map {
            let index_count = first_index_index_count_map
                .get(&first_index)
                .cloned()
                .unwrap_or_default();
            let submesh_folder_name = format!("{}-{}-{}", draw_ib, index_count, first_index);
            component_drawcall_index_list_dict
                .insert(submesh_folder_name, submesh_trianglelist_index_list);
        }
    }

    if component_drawcall_index_list_dict.is_empty() {
        return Ok(());
    }

    let deduped_folder_path = PathBuf::from(workspace_path).join("DedupedTextures");
    SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path)?;

    for index_list in component_drawcall_index_list_dict.values() {
        for trianglelist_index in index_list.iter() {
            let content_str = format!("{}-ps-t", trianglelist_index);
            let ps_texture_all_filename_list = fa.data.filter_texture_filename_list(&content_str);

            for ps_texture_filename in ps_texture_all_filename_list {
                let deduped_filepath = fa.log.get_deduped_filepath(&ps_texture_filename);
                if deduped_filepath.is_empty() {
                    continue;
                }

                let deduped_filename = fa.log.get_deduped_filename(&ps_texture_filename);
                let texture_unique_hash =
                    SSMTStringUtils::get_file_hash_from_file_name(&ps_texture_filename);

                let target_texture_path = deduped_folder_path
                    .join(format!("{}_{}", texture_unique_hash, deduped_filename));
                let target_texture_path_str = target_texture_path.to_string_lossy().to_string();
                let _ = SSMTFileUtils::copy_to_file_if_not_exists(
                    &deduped_filepath,
                    &target_texture_path_str,
                );
            }
        }
    }

    let component_json_path =
        get_workspace_component_name_draw_call_index_list_json_path(workspace_path)?;
    let component_json =
        ComponentNameDrawCallIndexListJson::from_map(component_drawcall_index_list_dict.clone());
    if let Err(e) = component_json.save_to_file(&component_json_path) {
        crate::extract_error!(
            "Failed to write {}: {}",
            component_json_path.to_string_lossy(),
            e
        );
    }

    let mut trianglelist_texture_file_name_list: Vec<String> = Vec::new();
    for index_list in component_drawcall_index_list_dict.values() {
        for trianglelist_index in index_list.iter() {
            let content_str = format!("{}-ps-t", trianglelist_index);
            let ps_texture_all_filename_list = fa.data.filter_texture_filename_list(&content_str);
            trianglelist_texture_file_name_list.extend(ps_texture_all_filename_list);
        }
    }

    let mut trianglelist_deduped_map: HashMap<String, TrianglelistDedupedTextureProperty> =
        HashMap::new();

    for trianglelist_texture_file_name in trianglelist_texture_file_name_list {
        let hash = SSMTStringUtils::get_file_hash_from_file_name(&trianglelist_texture_file_name);

        let fa_log_deduped_filename = {
            let deduped = fa.log.get_deduped_filename(&trianglelist_texture_file_name);
            if deduped.trim().is_empty() {
                String::new()
            } else {
                format!("{}_{}", hash, deduped)
            }
        };

        let fa_data_deduped_filename = {
            let mut out = String::new();
            let deduped_dir = Path::new(&fa.folder_path).join("deduped");
            if deduped_dir.exists() {
                if let Ok(entries) = fs::read_dir(&deduped_dir) {
                    for entry in entries.flatten() {
                        let file_name = entry.file_name();
                        if let Some(file_name_str) = file_name.to_str() {
                            if file_name_str.contains(&hash) {
                                out = format!("{}_{}", hash, file_name_str);
                                break;
                            }
                        }
                    }
                }
            }
            out
        };

        trianglelist_deduped_map.insert(
            trianglelist_texture_file_name,
            TrianglelistDedupedTextureProperty {
                fa_log_deduped_file_name: fa_log_deduped_filename.clone(),
                fa_data_deduped_file_name: fa_data_deduped_filename,
                format: SSMTStringUtils::extract_texture_format_from_deduped_file_name(
                    &fa_log_deduped_filename,
                ),
            },
        );
    }

    let trianglelist_json_path =
        get_workspace_trianglelist_deduped_filename_json_path(workspace_path)?;
    let trianglelist_json = TrianglelistDedupedFileNameJson::from_map(trianglelist_deduped_map);
    if let Err(e) = trianglelist_json.save_to_file(&trianglelist_json_path) {
        crate::extract_error!(
            "Failed to write {}: {}",
            trianglelist_json_path.to_string_lossy(),
            e
        );
    }

    Ok(())
}
