use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::frame_analysis::frameanalysis_data::FrameAnalysisData;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::common::vertex_buffer_txt_file::VertexBufferTxtFile;
use crate::config::drawib_config::{DrawIBConfig, DrawIBEntry};
use crate::config::path_manager::PathManager;
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::helper::workspace_texture_sync::sync_yysls_workspace_deduped_textures_and_json;
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::utils::ssmt_log_utils::SSMTLogUtils;
use crate::utils::ssmt_string_utils::SSMTStringUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshD3D11Element, SubMeshIndexBuffer, SubMeshJson,
};

pub struct YYSLSNewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

const SPECIAL_VS_HASH_8299998BC5A81A12: &str = "8299998bc5a81a12";
const SPECIAL_VS_REPLACE_PATH_8299998BC5A81A12: &str =
    r"c:\Users\Administrator\Desktop\8299998bc5a81a12-vs_replace.txt";

struct YyslsCtxExtractRequest {
    log_txt_file_path: PathBuf,
    ctx_data: FrameAnalysisData,
    trianglelist_index_list: Vec<String>,
    tmp_trianglelist_index: String,
    vb0_file_name: String,
    vs_hash: String,
}

struct YyslsVsCb0FieldOffsets {
    local_bounding_box_min: usize,
    local_bounding_box_max: usize,
    vertex_compression_params: usize,
}

impl YYSLSNewExtractor {
    fn parse_packoffset_register_offset(line: &str) -> Option<usize> {
        let start = line.find("packoffset(c")? + "packoffset(c".len();
        let register_digits: String = line[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if register_digits.is_empty() {
            return None;
        }

        let register_index = register_digits.parse::<usize>().ok()?;
        Some(register_index * 16)
    }

    fn default_8299998bc5a81a12_field_offsets() -> YyslsVsCb0FieldOffsets {
        YyslsVsCb0FieldOffsets {
            local_bounding_box_min: 18 * 16,
            local_bounding_box_max: 19 * 16,
            vertex_compression_params: 20 * 16,
        }
    }

    fn load_8299998bc5a81a12_field_offsets() -> Result<YyslsVsCb0FieldOffsets, String> {
        let shader_replace_path = Path::new(SPECIAL_VS_REPLACE_PATH_8299998BC5A81A12);
        if !shader_replace_path.exists() {
            return Ok(Self::default_8299998bc5a81a12_field_offsets());
        }

        let content = fs::read_to_string(shader_replace_path).map_err(|e| {
            format!(
                "Failed to read shader replace file {}: {}",
                shader_replace_path.to_string_lossy(),
                e
            )
        })?;

        let mut in_batch_cbuffer = false;
        let mut local_bounding_box_min = None;
        let mut local_bounding_box_max = None;
        let mut vertex_compression_params = None;

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("cbuffer Batch") {
                in_batch_cbuffer = true;
                continue;
            }

            if in_batch_cbuffer && trimmed.starts_with('}') {
                break;
            }

            if !in_batch_cbuffer {
                continue;
            }

            if trimmed.contains("LocalBoundingBoxMin") {
                local_bounding_box_min = Self::parse_packoffset_register_offset(trimmed);
            } else if trimmed.contains("LocalBoundingBoxMax") {
                local_bounding_box_max = Self::parse_packoffset_register_offset(trimmed);
            } else if trimmed.contains("VertexCompressionParams") {
                vertex_compression_params = Self::parse_packoffset_register_offset(trimmed);
            }
        }

        Ok(YyslsVsCb0FieldOffsets {
            local_bounding_box_min: local_bounding_box_min.unwrap_or(18 * 16),
            local_bounding_box_max: local_bounding_box_max.unwrap_or(19 * 16),
            vertex_compression_params: vertex_compression_params.unwrap_or(20 * 16),
        })
    }

    fn read_float4_from_buffer(
        buffer_bytes: &[u8],
        offset: usize,
        label: &str,
    ) -> Result<Vec<f32>, String> {
        let end = offset
            .checked_add(16)
            .ok_or_else(|| format!("{} offset overflow: offset={}", label, offset))?;
        if end > buffer_bytes.len() {
            return Err(format!(
                "{} range [{}..{}) exceeds vs-cb0 size {}",
                label,
                offset,
                end,
                buffer_bytes.len()
            ));
        }

        let mut values = Vec::with_capacity(4);
        for chunk in buffer_bytes[offset..end].chunks_exact(4) {
            values.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(values)
    }

    fn append_extra_fields_to_submesh_json(
        &self,
        json_path: &Path,
        extra_fields: &Map<String, Value>,
    ) -> Result<(), String> {
        let content = fs::read_to_string(json_path).map_err(|e| {
            format!(
                "Failed to read SubMeshJson {}: {}",
                json_path.to_string_lossy(),
                e
            )
        })?;
        let mut json_value: Value = serde_json::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse SubMeshJson {}: {}",
                json_path.to_string_lossy(),
                e
            )
        })?;
        let json_object = json_value.as_object_mut().ok_or_else(|| {
            format!(
                "SubMeshJson root is not an object: {}",
                json_path.to_string_lossy()
            )
        })?;

        for (key, value) in extra_fields {
            json_object.insert(key.clone(), value.clone());
        }

        let updated_content = serde_json::to_string_pretty(&json_value).map_err(|e| {
            format!(
                "Failed to serialize SubMeshJson {}: {}",
                json_path.to_string_lossy(),
                e
            )
        })?;
        fs::write(json_path, updated_content).map_err(|e| {
            format!(
                "Failed to write SubMeshJson {}: {}",
                json_path.to_string_lossy(),
                e
            )
        })
    }

    fn append_8299998bc5a81a12_fields_to_submesh_json(
        &self,
        request: &YyslsCtxExtractRequest,
        trianglelist_index: &str,
        json_path: &Path,
        field_offsets: &YyslsVsCb0FieldOffsets,
    ) -> Result<(), String> {
        let vs_cb0_key = format!("{}-vs-cb0=", trianglelist_index);
        let vs_cb0_filename = request
            .ctx_data
            .filter_first_file(&vs_cb0_key, ".buf")
            .unwrap_or_default();
        if vs_cb0_filename.is_empty() {
            return Err(format!(
                "vs-cb0 filename not found for TrianglelistIndex {}",
                trianglelist_index
            ));
        }

        let vs_cb0_file_path = Self::get_deduped_filepath_by_ctx_log(
            &PathBuf::from(&self.fa.folder_path),
            &request.log_txt_file_path,
            &vs_cb0_filename,
        )?;
        if vs_cb0_file_path.is_empty() {
            return Err(format!(
                "vs-cb0 deduped file path is empty for TrianglelistIndex {}: {}",
                trianglelist_index, vs_cb0_filename
            ));
        }

        let buffer_bytes = fs::read(&vs_cb0_file_path)
            .map_err(|e| format!("Failed to read vs-cb0 file {}: {}", vs_cb0_file_path, e))?;

        let mut extra_fields = Map::new();
        extra_fields.insert(
            "LocalBoundingBoxMin".to_string(),
            json!(Self::read_float4_from_buffer(
                &buffer_bytes,
                field_offsets.local_bounding_box_min,
                "LocalBoundingBoxMin",
            )?),
        );
        extra_fields.insert(
            "LocalBoundingBoxMax".to_string(),
            json!(Self::read_float4_from_buffer(
                &buffer_bytes,
                field_offsets.local_bounding_box_max,
                "LocalBoundingBoxMax",
            )?),
        );
        extra_fields.insert(
            "VertexCompressionParams".to_string(),
            json!(Self::read_float4_from_buffer(
                &buffer_bytes,
                field_offsets.vertex_compression_params,
                "VertexCompressionParams",
            )?),
        );

        self.append_extra_fields_to_submesh_json(json_path, &extra_fields)
    }

    fn build_category_hash_from_buf_file_name(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
        buf_file_name: &str,
    ) -> Result<String, String> {
        let category_slot = d3d11_game_type
            .category_slot_dict
            .get(category_name)
            .ok_or_else(|| format!("Category slot not found for category: {}", category_name))?;
        let start_index = 8usize + category_slot.len();
        let hash: String = buf_file_name.chars().skip(start_index).take(8).collect();
        if hash.len() != 8 {
            return Err(format!(
                "Cannot parse hash from buf file name: {} (category={}, slot={})",
                buf_file_name, category_name, category_slot
            ));
        }
        Ok(hash)
    }

    fn build_submesh_elements_for_category(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
    ) -> Vec<SubMeshD3D11Element> {
        let mut result = Vec::new();
        for element_name in &d3d11_game_type.ordered_full_element_list {
            let Some(element) = d3d11_game_type
                .element_name_d3d11_element_dict
                .get(element_name)
            else {
                continue;
            };
            if element.category == category_name {
                result.push(SubMeshD3D11Element::from_d3d11_element(element));
            }
        }
        result
    }

    fn export_category_buffer<F>(
        &self,
        category_name: &str,
        category_buf_filename: &str,
        category_output_buf_file_path: &Path,
        resolve_path: &F,
    ) -> Result<(), String>
    where
        F: Fn(&str) -> String,
    {
        let category_buf_file_path = resolve_path(category_buf_filename);
        if category_buf_file_path.is_empty() {
            return Err(format!(
                "Category {} deduped path is empty: {}",
                category_name, category_buf_filename
            ));
        }

        let category_txt_filename =
            SSMTFileUtils::get_filename_with_new_extension(category_buf_filename, "txt")?;
        let category_txt_file_path = resolve_path(&category_txt_filename);

        if !category_txt_file_path.is_empty() && Path::new(&category_txt_file_path).exists() {
            let metadata = SSMTBinaryUtils::read_migoto_buffer_metadata(&category_txt_file_path)?;
            if metadata.stride > 0 && metadata.vertex_count > 0 {
                let category_buf_bytes = fs::read(&category_buf_file_path).map_err(|e| {
                    format!(
                        "Failed to read category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                let read_len = metadata
                    .vertex_count
                    .checked_mul(metadata.stride)
                    .ok_or_else(|| {
                        format!(
                            "Category {} slice length overflow: vertex_count={} stride={}",
                            category_name, metadata.vertex_count, metadata.stride
                        )
                    })?;
                let end = metadata.byte_offset.checked_add(read_len).ok_or_else(|| {
                    format!(
                        "Category {} slice end overflow: byte_offset={} read_len={}",
                        category_name, metadata.byte_offset, read_len
                    )
                })?;
                let sliced_bytes = SSMTBinaryUtils::get_range_bytes(
                    &category_buf_bytes,
                    metadata.byte_offset,
                    end,
                )
                .map_err(|e| {
                    format!(
                        "Failed to slice category buffer for category {}: {}",
                        category_name, e
                    )
                })?;

                fs::write(category_output_buf_file_path, sliced_bytes).map_err(|e| {
                    format!(
                        "Failed to write category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                return Ok(());
            }
        }

        fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
            format!(
                "Failed to copy category buffer file for category {}: {}",
                category_name, e
            )
        })?;

        Ok(())
    }

    fn export_single_submesh<F>(
        &self,
        game_preset: &str,
        draw_ib: &str,
        vertex_limit_vb: &str,
        d3d11_game_type: &D3D11GameType,
        category_buf_filename_map: &HashMap<String, String>,
        ib_txt_file_name: &str,
        resolve_path: F,
    ) -> Result<PathBuf, String>
    where
        F: Fn(&str) -> String,
    {
        let ib_buf_file_name =
            SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
        let ib_txt_file_path = resolve_path(ib_txt_file_name);
        let ib_buf_file_path = resolve_path(&ib_buf_file_name);
        if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
            return Ok(PathBuf::new());
        }
        let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
        let unique_str_folder_name = format!(
            "{}-{}-{}",
            draw_ib, ib_txt_file.index_count, ib_txt_file.first_index
        );
        let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);
        let game_type_output_path = PathBuf::from(&self.workspace_path)
            .join(&unique_str_folder_name)
            .join(&game_type_folder_name);
        SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;
        let name_prefix = unique_str_folder_name.clone();
        let output_ib_buf_file_path = game_type_output_path.join(format!("{}.ib", name_prefix));
        let mut ib_buf_file =
            IndexBufferBufFile::from_file(&ib_buf_file_path, &ib_txt_file.format)?;
        ib_buf_file.self_divide(
            ib_txt_file.first_index.parse::<usize>().unwrap_or(0),
            ib_txt_file.index_number_count as usize,
        );
        ib_buf_file.save_to_file_uint32(&output_ib_buf_file_path, 0)?;

        for category_name in &d3d11_game_type.ordered_category_name_list {
            let category_buf_filename = category_buf_filename_map
                .get(category_name)
                .cloned()
                .unwrap_or_default();
            let category_output_buf_file_path =
                game_type_output_path.join(format!("{}-{}.buf", name_prefix, category_name));
            self.export_category_buffer(
                category_name,
                &category_buf_filename,
                &category_output_buf_file_path,
                &resolve_path,
            )?;
        }
        let mut submesh_json = SubMeshJson::new();
        submesh_json.game_preset = game_preset.to_string();
        submesh_json.vertex_limit_vb = vertex_limit_vb.to_string();
        submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
        submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
        submesh_json.index_buffer_list.push(SubMeshIndexBuffer {
            dxgi_format: "DXGI_FORMAT_R32_UINT".to_string(),
            file_name: format!("{}.ib", name_prefix),
        });
        for category_name in &d3d11_game_type.ordered_category_name_list {
            let category_buf_filename = category_buf_filename_map
                .get(category_name)
                .cloned()
                .unwrap_or_default();
            let category_hash = self.build_category_hash_from_buf_file_name(
                d3d11_game_type,
                category_name,
                &category_buf_filename,
            )?;
            submesh_json
                .category_hash_dict
                .insert(category_name.clone(), category_hash);
            submesh_json.category_draw_category_map.insert(
                category_name.clone(),
                d3d11_game_type
                    .category_draw_category_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default(),
            );
            submesh_json
                .category_buffer_list
                .push(SubMeshCategoryBuffer {
                    file_name: format!("{}-{}.buf", name_prefix, category_name),
                    buffer_type: "Normal".to_string(),
                    d3d11_element_list: self
                        .build_submesh_elements_for_category(d3d11_game_type, category_name),
                });
        }
        let submesh_json_path = game_type_output_path.join(name_prefix.to_string() + ".json");
        submesh_json.save_to_file(&submesh_json_path)?;
        Ok(submesh_json_path)
    }

    pub fn new(
        frame_analysis_folder: &String,
        workspace_path: &String,
        is_full_extract: bool,
    ) -> Result<Self, String> {
        Self::new_internal(frame_analysis_folder, workspace_path, !is_full_extract)
    }

    fn new_internal(
        frame_analysis_folder: &String,
        workspace_path: &String,
        specify_drawib_extract: bool,
    ) -> Result<Self, String> {
        let frame_analysis_dir = PathBuf::from(frame_analysis_folder);
        if !frame_analysis_dir.exists() {
            return Err(format!(
                "FrameAnalysis 文件夹未找到: {}",
                frame_analysis_folder
            ));
        }

        let fa = FrameAnalysis::new(frame_analysis_folder)?;
        let drawib_config = if specify_drawib_extract {
            DrawIBConfig::new_from_workspace(workspace_path)
                .map_err(|e| format!("Failed to read DrawIB config: {}", e))?
        } else {
            DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            }
        };

        let gametype_folder_path = PathManager::bundled_gametype_folder();
        let current_gametype_folder_path = gametype_folder_path.join("YYSLS");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract,
            d3d11_gametype_lv2,
        })
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

    fn get_deduped_filepath_by_ctx_log(
        frame_analysis_folder: &Path,
        ctx_log_file_path: &Path,
        frame_analysis_filename: &str,
    ) -> Result<String, String> {
        let map = Self::parse_filename_deduped_map_from_log_file(ctx_log_file_path)?;
        let deduped_file_name = map
            .get(frame_analysis_filename)
            .cloned()
            .unwrap_or_default();

        if deduped_file_name.is_empty() {
            return Ok(String::new());
        }

        let deduped_path = frame_analysis_folder
            .join("deduped")
            .join(deduped_file_name);
        Ok(deduped_path.to_string_lossy().to_string())
    }

    fn build_ctx_extract_requests_for_draw_ib(
        &self,
        draw_ib: &str,
    ) -> Result<Vec<YyslsCtxExtractRequest>, String> {
        let frame_analysis_folder = PathBuf::from(&self.fa.folder_path);
        let mut log_txt_file_path_list: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(&frame_analysis_folder).map_err(|e| e.to_string())? {
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

        let mut request_list: Vec<YyslsCtxExtractRequest> = Vec::new();
        for log_txt_file_path in &log_txt_file_path_list {
            let raw = fs::read(log_txt_file_path).map_err(|e| {
                format!(
                    "Failed to read log file {}: {}",
                    log_txt_file_path.to_string_lossy(),
                    e
                )
            })?;
            let content = String::from_utf8_lossy(&raw).into_owned();

            if content
                .lines()
                .any(|line| line.contains(&format!("hash={}", draw_ib)))
            {
                crate::extract_log!(
                    "当前DrawIB在【{}】中存在",
                    log_txt_file_path.to_string_lossy()
                );
            } else {
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

            crate::extract_log!("CTXCode: {}", ctx_code);

            let ctx_folder_name = format!("ctx-{}", ctx_code);
            let ctx_folder_path = frame_analysis_folder.join(&ctx_folder_name);
            crate::extract_log!("CTXFolderPath: {}", ctx_folder_path.to_string_lossy());

            if !ctx_folder_path.exists() || !ctx_folder_path.is_dir() {
                crate::extract_log!(
                    "当前CTX文件夹不存在，跳过: {}",
                    ctx_folder_path.to_string_lossy()
                );
                continue;
            }

            let ctx_data = FrameAnalysisData::new(&ctx_folder_path.to_string_lossy())?;

            let trianglelist_ib_file_list =
                ctx_data.filter_filelist(&format!("-ib={}", draw_ib), ".txt");

            let mut trianglelist_index_list: Vec<String> = Vec::new();
            crate::extract_log!("TrianglelistIndexList:");
            for trianglelist_ib_file_name in &trianglelist_ib_file_list {
                let index = trianglelist_ib_file_name
                    .get(0..6)
                    .unwrap_or_default()
                    .to_string();
                if !index.is_empty() {
                    trianglelist_index_list.push(index.clone());
                    crate::extract_log!("Index: {}", index);
                }
            }

            if trianglelist_index_list.is_empty() {
                crate::extract_log!("当前CTX文件夹中未找到任何DrawIB相关文件，跳过提取");
                continue;
            }

            let tmp_trianglelist_index =
                trianglelist_index_list.last().cloned().unwrap_or_default();
            let vb0_file_name = ctx_data
                .filter_first_file(&format!("{}-vb0", tmp_trianglelist_index), ".txt")
                .unwrap_or_default();

            let vs_hash = SSMTStringUtils::get_vs_hash_from_file_name(&vb0_file_name);
            request_list.push(YyslsCtxExtractRequest {
                log_txt_file_path: log_txt_file_path.clone(),
                ctx_data,
                trianglelist_index_list,
                tmp_trianglelist_index,
                vb0_file_name,
                vs_hash,
            });
        }

        Ok(request_list)
    }

    fn detect_possible_game_types_for_request(
        &self,
        draw_ib: &str,
        request: &YyslsCtxExtractRequest,
    ) -> Result<Vec<D3D11GameType>, String> {
        let frame_analysis_folder = PathBuf::from(&self.fa.folder_path);
        crate::extract_log!("tmpTrianglelistIndex: {}", request.tmp_trianglelist_index);

        let mut possible_game_type_list: Vec<D3D11GameType> = Vec::new();
        for d3d11_game_type in &self.d3d11_gametype_lv2.ordered_gpu_cpu_d3d11_gametype_list {
            crate::extract_log!("尝试匹配数据类型: {}", d3d11_game_type.game_type_name);

            let mut category_name_buf_file_path_dict: HashMap<String, String> = HashMap::new();
            let mut all_slot_buf_file_exists = true;

            for (category_name, category_slot) in &d3d11_game_type.category_slot_dict {
                let category_buf_file_name = request
                    .ctx_data
                    .filter_first_file(
                        &format!("{}-{}=", request.tmp_trianglelist_index, category_slot),
                        ".buf",
                    )
                    .unwrap_or_default();
                if category_buf_file_name.is_empty() {
                    all_slot_buf_file_exists = false;
                    break;
                }

                let category_buf_file_path = Self::get_deduped_filepath_by_ctx_log(
                    &frame_analysis_folder,
                    &request.log_txt_file_path,
                    &category_buf_file_name,
                )?;
                if category_buf_file_path.is_empty() {
                    all_slot_buf_file_exists = false;
                    break;
                }

                category_name_buf_file_path_dict
                    .insert(category_name.clone(), category_buf_file_path);
            }

            if !all_slot_buf_file_exists {
                crate::extract_log!("当前数据类型并非所有的槽位Buffer文件都存在，不满足，跳过。");
                continue;
            }

            let mut vertex_number: u64 = 0;
            let mut all_match = true;

            for category_name in &d3d11_game_type.ordered_category_name_list {
                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                let category_stride = d3d11_game_type
                    .category_stride_dict
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);

                crate::extract_log!(
                    "当前匹配槽位: {} Stride: {}",
                    category_name,
                    category_stride
                );

                if category_stride == 0 {
                    all_match = false;
                    break;
                }

                let buf_file_path = category_name_buf_file_path_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                if buf_file_path.is_empty() {
                    all_match = false;
                    break;
                }

                let category_buf_txt_file_name = request
                    .ctx_data
                    .filter_first_file(
                        &format!("{}-{}=", request.tmp_trianglelist_index, category_slot),
                        ".txt",
                    )
                    .unwrap_or_default();
                let category_buf_txt_file_path = Self::get_deduped_filepath_by_ctx_log(
                    &frame_analysis_folder,
                    &request.log_txt_file_path,
                    &category_buf_txt_file_name,
                )?;
                if !category_buf_txt_file_name.is_empty() && category_buf_txt_file_path.is_empty() {
                    return Err(format!(
                        "Category {} txt file path is empty: {}",
                        category_name, category_buf_txt_file_name
                    ));
                }

                let buf_file_size;
                if category_buf_txt_file_name.is_empty() || category_buf_txt_file_path.is_empty() {
                    buf_file_size = SSMTFileUtils::get_file_size(&buf_file_path)?;
                } else {
                    let category_txt_file = VertexBufferTxtFile::new(&category_buf_txt_file_path)?;
                    if category_txt_file.vertex_count.is_empty() {
                        buf_file_size = SSMTFileUtils::get_file_size(&buf_file_path)?;
                    } else {
                        let category_stride_int =
                            category_txt_file.stride.parse::<u64>().unwrap_or(0);
                        let vertex_count_int =
                            category_txt_file.vertex_count.parse::<u64>().unwrap_or(0);
                        buf_file_size = vertex_count_int * category_stride_int;
                    }
                }

                let tmp_number = buf_file_size / category_stride;
                let yu_shu = buf_file_size % category_stride;
                if yu_shu != 0 {
                    crate::extract_log!("余数不为0: {}，文件步长除以类别步长不能含余数", yu_shu);
                    all_match = false;
                    break;
                }
                if vertex_number == 0 {
                    vertex_number = tmp_number;
                }
                if tmp_number == 0 {
                    crate::extract_log!("当前匹配的槽位文件大小为0: {}", buf_file_path);
                    all_match = false;
                    break;
                }
                if vertex_number != tmp_number {
                    crate::extract_log!(
                        "VertexNumber: {} 当前槽位数量: {}",
                        vertex_number,
                        tmp_number
                    );
                    crate::extract_log!("槽位匹配失败");
                    all_match = false;
                    break;
                }
            }

            if all_match {
                possible_game_type_list.push(d3d11_game_type.clone());
            }
        }

        if possible_game_type_list.is_empty() {
            crate::extract_new::log_skipped_drawib(
                draw_ib,
                format!(
                    "no data type matched. TrianglelistIndex: {:?}",
                    request.trianglelist_index_list
                ),
            );
            return Ok(Vec::new());
        }

        crate::extract_log!("当前匹配到的数据类型列表:");
        for d3d11_game_type in &possible_game_type_list {
            crate::extract_log!("{}", d3d11_game_type.game_type_name);
        }

        Ok(possible_game_type_list)
    }

    fn build_match_first_index_ib_file_name_dict(
        &self,
        request: &YyslsCtxExtractRequest,
    ) -> Result<BTreeMap<u64, String>, String> {
        let frame_analysis_folder = PathBuf::from(&self.fa.folder_path);
        let mut match_first_index_ib_file_name_dict: BTreeMap<u64, String> = BTreeMap::new();

        for trianglelist_index in &request.trianglelist_index_list {
            let ib_txt_file_name = request
                .ctx_data
                .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                .unwrap_or_default();
            if ib_txt_file_name.is_empty() {
                continue;
            }

            let ib_file_path = Self::get_deduped_filepath_by_ctx_log(
                &frame_analysis_folder,
                &request.log_txt_file_path,
                &ib_txt_file_name,
            )?;
            if ib_file_path.is_empty() || !Path::new(&ib_file_path).exists() {
                continue;
            }

            let ib_txt_file = IndexBufferTxtFile::new(&ib_file_path, false)?;
            let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            match_first_index_ib_file_name_dict
                .entry(match_first_index)
                .or_insert(ib_txt_file_name);
        }

        Ok(match_first_index_ib_file_name_dict)
    }

    fn extract_ctx_request(
        &self,
        draw_ib: &str,
        request: &YyslsCtxExtractRequest,
        field_offsets: Option<&YyslsVsCb0FieldOffsets>,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<bool, String> {
        let frame_analysis_folder = PathBuf::from(&self.fa.folder_path);
        let mut possible_game_type_list =
            self.detect_possible_game_types_for_request(draw_ib, request)?;
        possible_game_type_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));
        let match_first_index_ib_file_name_dict =
            self.build_match_first_index_ib_file_name_dict(request)?;

        for d3d11_game_type in &possible_game_type_list {
            crate::extract_log!("当前提取数据类型: {}", d3d11_game_type.game_type_name);

            for ib_txt_file_name in match_first_index_ib_file_name_dict.values() {
                let output_vb_index = ib_txt_file_name.get(0..6).unwrap_or_default().to_string();
                let ib_txt_file_path = Self::get_deduped_filepath_by_ctx_log(
                    &frame_analysis_folder,
                    &request.log_txt_file_path,
                    ib_txt_file_name,
                )?;
                if ib_txt_file_path.is_empty() {
                    continue;
                }

                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                let ib_buf_file_path = Self::get_deduped_filepath_by_ctx_log(
                    &frame_analysis_folder,
                    &request.log_txt_file_path,
                    &ib_buf_file_name,
                )?;
                if ib_buf_file_path.is_empty() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
                crate::extract_log!("{}", ib_txt_file_path);
                crate::extract_log!("FirstIndex: {}", ib_txt_file.first_index);
                crate::extract_log!("IndexCount: {}", ib_txt_file.index_count);

                crate::extract_log!("开始从各个Buffer文件中读取数据:");
                let mut category_name_buf_file_name_dict: HashMap<String, String> = HashMap::new();
                for category_name in &d3d11_game_type.ordered_category_name_list {
                    let category_stride = d3d11_game_type
                        .category_stride_dict
                        .get(category_name)
                        .copied()
                        .unwrap_or(0);
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();

                    let category_buf_file_name = request
                        .ctx_data
                        .filter_first_file(
                            &format!("{}-{}=", output_vb_index, category_slot),
                            ".buf",
                        )
                        .unwrap_or_default();
                    if !category_buf_file_name.is_empty() {
                        category_name_buf_file_name_dict
                            .insert(category_name.clone(), category_buf_file_name.clone());
                    }

                    let category_buf_file_path = Self::get_deduped_filepath_by_ctx_log(
                        &frame_analysis_folder,
                        &request.log_txt_file_path,
                        &category_buf_file_name,
                    )?;
                    if category_buf_file_path.is_empty() || category_stride == 0 {
                        return Err(format!(
                            "Category {} 路径或 stride 无效: path={} stride={}",
                            category_name, category_buf_file_path, category_stride
                        ));
                    }
                }

                let submesh_json_path = self.export_single_submesh(
                    "YYSLS",
                    draw_ib,
                    &if request.vb0_file_name.is_empty() {
                        String::new()
                    } else {
                        request.vb0_file_name.chars().skip(11).take(8).collect()
                    },
                    d3d11_game_type,
                    &category_name_buf_file_name_dict,
                    ib_txt_file_name,
                    |file_name| {
                        Self::get_deduped_filepath_by_ctx_log(
                            &frame_analysis_folder,
                            &request.log_txt_file_path,
                            file_name,
                        )
                        .unwrap_or_default()
                    },
                )?;

                if let Some(offsets) = field_offsets {
                    if !submesh_json_path.as_os_str().is_empty() {
                        self.append_8299998bc5a81a12_fields_to_submesh_json(
                            request,
                            &output_vb_index,
                            &submesh_json_path,
                            offsets,
                        )?;
                    }
                }
            }
        }

        Ok(true)
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        crate::extract_log!("开始提取:");

        let draw_ib_list = if self.specify_drawib_extract {
            self.drawib_config
                .entries
                .iter()
                .map(|entry| entry.draw_ib.trim().to_string())
                .filter(|draw_ib| !draw_ib.is_empty())
                .collect::<Vec<String>>()
        } else {
            self.fa.data.get_all_drawib_list()
        };

        for draw_ib in draw_ib_list.iter() {
            crate::extract_log!("当前DrawIB: {}", draw_ib);

            let request_list = self.build_ctx_extract_requests_for_draw_ib(draw_ib)?;
            if request_list.is_empty() {
                crate::extract_new::log_skipped_drawib(draw_ib, "no available CTX extract request");
                continue;
            }

            SSMTLogUtils::seperator();
            for request in request_list.iter() {
                crate::extract_log!("当前CTX对应的VSHash: {}", request.vs_hash);

                let result = if request.vs_hash == SPECIAL_VS_HASH_8299998BC5A81A12 {
                    crate::extract_log!("执行提取流程: extract_8299998bc5a81a12");
                    let field_offsets = Self::load_8299998bc5a81a12_field_offsets()?;
                    self.extract_ctx_request(
                        draw_ib,
                        request,
                        Some(&field_offsets),
                        data_type_filter,
                    )?
                } else {
                    crate::extract_log!("执行提取流程: extract_default_ctx_request");
                    self.extract_ctx_request(draw_ib, request, None, data_type_filter)?
                };

                if !result {
                    crate::extract_new::log_skipped_drawib(
                        draw_ib,
                        "CTX extract request produced no output",
                    );
                    continue;
                }
            }
        }

        crate::extract_log!("提取正常执行完成");
        if self.specify_drawib_extract {
            sync_yysls_workspace_deduped_textures_and_json(
                &self.fa,
                &self.drawib_config,
                &self.workspace_path,
            )?;
        } else {
            let full_drawib_config = DrawIBConfig {
                path: String::new(),
                entries: draw_ib_list
                    .into_iter()
                    .map(|draw_ib| DrawIBEntry {
                        draw_ib: draw_ib.clone(),
                        alias: draw_ib,
                    })
                    .collect(),
            };
            sync_yysls_workspace_deduped_textures_and_json(
                &self.fa,
                &full_drawib_config,
                &self.workspace_path,
            )?;
        }
        Ok(())
    }
}
