use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::common::vertex_buffer_txt_file::VertexBufferTxtFile;
use crate::config::drawib_config::{DrawIBConfig, DrawIBEntry};
use crate::config::path_manager::PathManager;
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::helper::workspace_texture_sync::sync_workspace_deduped_textures_and_json;
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshD3D11Element, SubMeshIndexBuffer, SubMeshJson,
};
use std::fs;

pub struct APMI2NewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl APMI2NewExtractor {
    fn copy_dump_to_apmi_snapshot(
        &self,
        logical_filename: &str,
        snapshot_dir: &Path,
    ) -> Result<bool, String> {
        if logical_filename.trim().is_empty() {
            return Ok(false);
        }
        let source_path = self.fa.log.get_deduped_filepath(logical_filename);
        if source_path.is_empty() || !Path::new(&source_path).is_file() {
            return Ok(false);
        }
        fs::copy(&source_path, snapshot_dir.join(logical_filename)).map_err(|error| {
            format!(
                "Failed to copy APMI capture resource {}: {}",
                logical_filename, error
            )
        })?;
        Ok(true)
    }

    fn copy_apmi_ordinal_resources(
        &self,
        ordinal: &str,
        slots: &[&str],
        txt_slots: &[&str],
        snapshot_dir: &Path,
    ) -> Result<(), String> {
        for slot in slots {
            let search = format!("{}-{}=", ordinal, slot);
            let buf_filename = self
                .fa
                .data
                .filter_first_file(&search, ".buf")
                .unwrap_or_default();
            self.copy_dump_to_apmi_snapshot(&buf_filename, snapshot_dir)?;
            if txt_slots.contains(slot) {
                let txt_filename = self
                    .fa
                    .data
                    .filter_first_file(&search, ".txt")
                    .unwrap_or_default();
                self.copy_dump_to_apmi_snapshot(&txt_filename, snapshot_dir)?;
            }
        }
        Ok(())
    }

    fn write_workspace_apmi_capture_snapshot(
        &self,
        extracted_draw_ib_list: &BTreeSet<String>,
    ) -> Result<(), String> {
        if extracted_draw_ib_list.is_empty() {
            return Ok(());
        }

        let snapshot_dir = PathBuf::from(&self.workspace_path).join("APMICapture");
        SSMTFileUtils::create_folder_if_not_exists(&snapshot_dir)?;

        let mut pointlist_indices = BTreeSet::new();
        let mut trianglelist_indices = BTreeSet::new();
        for draw_ib in extracted_draw_ib_list {
            if let Some(pointlist_index) = self.fa.log.get_last_pointlist_index_by_hash(draw_ib) {
                if !pointlist_index.is_empty() {
                    pointlist_indices.insert(pointlist_index);
                }
            }
            for trianglelist_index in self.fa.data.get_trianglelist_index_list(draw_ib) {
                if !trianglelist_index.is_empty() {
                    trianglelist_indices.insert(trianglelist_index);
                }
            }
        }

        for ordinal in &pointlist_indices {
            self.copy_apmi_ordinal_resources(
                ordinal,
                &["cs-t0", "cs-t1", "cs-cb0", "cs-cb1", "cs-t2", "u0"],
                &[],
                &snapshot_dir,
            )?;
        }
        for ordinal in &trianglelist_indices {
            self.copy_apmi_ordinal_resources(
                ordinal,
                &["ib", "vb0", "vb1", "vb2", "vb3"],
                &["vb1"],
                &snapshot_dir,
            )?;
        }

        let log_path = snapshot_dir.join("log.txt");
        let mut log_lines = if log_path.is_file() {
            fs::read_to_string(&log_path)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        } else {
            BTreeSet::new()
        };
        let ordinals = pointlist_indices
            .iter()
            .chain(trianglelist_indices.iter())
            .collect::<BTreeSet<_>>();
        for ordinal in ordinals {
            for line in self.fa.log.get_line_list_by_index(ordinal) {
                if line.contains(" Dispatch(") || line.contains(" DrawIndexed(") {
                    log_lines.insert(line);
                }
            }
        }
        let log_text = log_lines.into_iter().collect::<Vec<_>>().join("\n");
        fs::write(log_path, log_text).map_err(|error| {
            format!(
                "Failed to write workspace APMI capture log {}: {}",
                snapshot_dir.display(),
                error
            )
        })?;
        Ok(())
    }

    fn build_vertex_limit_vb(&self, trianglelist_index: &str) -> String {
        let vb0_file_name = self
            .fa
            .data
            .filter_first_file(&format!("{}-vb0", trianglelist_index), ".txt")
            .unwrap_or_default();

        if vb0_file_name.is_empty() {
            String::new()
        } else {
            vb0_file_name.chars().skip(11).take(8).collect()
        }
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
        self.build_slot_hash_from_buf_file_name(category_name, category_slot, buf_file_name)
    }

    fn build_slot_hash_from_buf_file_name(
        &self,
        label: &str,
        slot_name: &str,
        buf_file_name: &str,
    ) -> Result<String, String> {
        let start_index = 8usize + slot_name.len();
        let hash: String = buf_file_name.chars().skip(start_index).take(8).collect();
        if hash.len() != 8 {
            return Err(format!(
                "Cannot parse hash from buf file name: {} (label={}, slot={})",
                buf_file_name, label, slot_name
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

    fn build_source_texcoord_elements(
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
        vertex_buffer: &VertexBufferTxtFile,
    ) -> Result<Vec<SubMeshD3D11Element>, String> {
        let stride = vertex_buffer
            .stride
            .parse::<u64>()
            .map_err(|_| format!("APMI: invalid vb1 stride: {}", vertex_buffer.stride))?;
        let extract_slot = d3d11_game_type
            .category_slot_dict
            .get(category_name)
            .cloned()
            .unwrap_or_default();
        let expected_input_slot = extract_slot
            .strip_prefix("vb")
            .and_then(|value| value.parse::<u64>().ok());
        let extract_technique = if vertex_buffer.topology.is_empty() {
            d3d11_game_type
                .category_topology_dict
                .get(category_name)
                .cloned()
                .unwrap_or_default()
        } else {
            vertex_buffer.topology.clone()
        };
        let draw_category = d3d11_game_type
            .category_draw_category_dict
            .get(category_name)
            .cloned()
            .unwrap_or_default();

        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for element in &vertex_buffer.d3d11_element_list {
            if let (Some(expected_slot), Some(actual_slot)) = (
                expected_input_slot,
                vertex_buffer
                    .input_slot_by_element_name
                    .get(&element.element_name),
            ) {
                if *actual_slot != expected_slot {
                    continue;
                }
            }
            if !seen.insert(element.element_name.clone()) {
                continue;
            }
            let offset = vertex_buffer
                .aligned_byte_offset_by_element_name
                .get(&element.element_name)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "APMI: vb1 element {} is missing AlignedByteOffset",
                        element.element_name
                    )
                })?;
            let byte_width = element.byte_width_int();
            if byte_width == 0 || offset.saturating_add(byte_width) > stride {
                return Err(format!(
                    "APMI: vb1 element {} is outside stride {} (offset={}, width={})",
                    element.element_name, stride, offset, byte_width
                ));
            }
            result.push(SubMeshD3D11Element {
                semantic_name: element.semantic_name.clone(),
                semantic_index: element.semantic_index.to_string(),
                format: element.format.clone(),
                byte_width: element.byte_width.clone(),
                aligned_byte_offset: offset.to_string(),
                extract_slot: extract_slot.clone(),
                extract_technique: extract_technique.clone(),
                category: category_name.to_string(),
                draw_category: draw_category.clone(),
            });
        }

        if !result
            .iter()
            .any(|element| element.semantic_name.eq_ignore_ascii_case("TEXCOORD"))
        {
            return Err("APMI: vb1 layout does not contain TEXCOORD".to_string());
        }
        Ok(result)
    }

    fn build_workspace_elements_for_category(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
        category_txt_filename: &str,
    ) -> Result<Vec<SubMeshD3D11Element>, String> {
        if category_name == "Texcoord" && !category_txt_filename.is_empty() {
            let category_txt_path = self.fa.log.get_deduped_filepath(category_txt_filename);
            if !category_txt_path.is_empty() && Path::new(&category_txt_path).is_file() {
                let vertex_buffer = VertexBufferTxtFile::new(&category_txt_path)?;
                return Self::build_source_texcoord_elements(
                    d3d11_game_type,
                    category_name,
                    &vertex_buffer,
                );
            }
        }
        Ok(self.build_submesh_elements_for_category(d3d11_game_type, category_name))
    }

    fn export_category_buffer(
        &self,
        category_name: &str,
        category_buf_filename: &str,
        category_txt_filename: Option<&str>,
        gpu_pre_skinning: bool,
        category_output_buf_file_path: &Path,
    ) -> Result<(), String> {
        let category_buf_file_path = self.fa.log.get_deduped_filepath(category_buf_filename);
        if category_buf_file_path.is_empty() {
            return Err(format!(
                "Category {} deduped path is empty: {}",
                category_name, category_buf_filename
            ));
        }

        if gpu_pre_skinning {
            fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
                format!(
                    "Failed to copy category buffer file for category {}: {}",
                    category_name, e
                )
            })?;
            return Ok(());
        }

        let fallback_txt_filename;
        let effective_txt_filename = match category_txt_filename {
            Some(file_name) if !file_name.trim().is_empty() => file_name,
            _ => {
                fallback_txt_filename =
                    SSMTFileUtils::get_filename_with_new_extension(category_buf_filename, "txt")?;
                fallback_txt_filename.as_str()
            }
        };

        let category_txt_file_path = self.fa.log.get_deduped_filepath(effective_txt_filename);
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

    fn collect_match_first_index_ib_map(
        &self,
        trianglelist_index_list: &[String],
    ) -> Result<BTreeMap<u64, String>, String> {
        let mut result: BTreeMap<u64, String> = BTreeMap::new();
        for trianglelist_index in trianglelist_index_list {
            let ib_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                .unwrap_or_default();
            if ib_file_name.is_empty() {
                continue;
            }
            let ib_file_path = self.fa.log.get_deduped_filepath(&ib_file_name);
            if ib_file_path.is_empty() {
                continue;
            }
            let ib_txt_file = IndexBufferTxtFile::new(&ib_file_path, false)?;
            let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            result.insert(match_first_index, ib_file_name);
        }
        Ok(result)
    }

    fn export_unity_vs_submeshes(
        &self,
        game_preset: &str,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
        possible_d3d11_game_type_list: &[D3D11GameType],
    ) -> Result<bool, String> {
        if possible_d3d11_game_type_list.is_empty() {
            return Ok(false);
        }
        let match_first_index_ib_txt_file_name_dict =
            self.collect_match_first_index_ib_map(trianglelist_index_list)?;
        for (match_first_index, ib_file_name) in &match_first_index_ib_txt_file_name_dict {
            crate::extract_log!(
                "MatchFirstIndex: {} IBFileName: {}",
                match_first_index,
                ib_file_name
            );
        }
        for d3d11_game_type in possible_d3d11_game_type_list {
            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa.data,
                trianglelist_index_list,
                d3d11_game_type,
            );
            let vertex_limit_vb = self.build_vertex_limit_vb(&trianglelist_index);
            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);
            for ib_txt_file_name in match_first_index_ib_txt_file_name_dict.values() {
                let per_ib_trianglelist_index: String = ib_txt_file_name.chars().take(6).collect();
                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                let ib_txt_file_path = self.fa.log.get_deduped_filepath(ib_txt_file_name);
                let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);
                if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
                    continue;
                }
                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
                let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
                let mut category_txt_filename_map: HashMap<String, String> = HashMap::new();
                for category_name in &d3d11_game_type.ordered_category_name_list {
                    let topology = d3d11_game_type
                        .category_topology_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let mut extract_index = per_ib_trianglelist_index.clone();
                    if topology == "pointlist" && !pointlist_index.is_empty() {
                        extract_index = pointlist_index.to_string();
                    }
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_buf_file_name = self.fa.data.filter_first_file(
                        &format!("{}-{}", extract_index, category_slot),
                        ".buf",
                    )?;
                    let category_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(&format!("{}-{}", extract_index, category_slot), ".txt")
                        .unwrap_or_default();
                    category_buf_filename_map.insert(category_name.clone(), category_buf_file_name);
                    category_txt_filename_map.insert(category_name.clone(), category_txt_file_name);
                }
                let unique_str_folder_name = format!(
                    "{}-{}-{}",
                    draw_ib, ib_txt_file.index_count, ib_txt_file.first_index
                );
                let game_type_output_path = PathBuf::from(&self.workspace_path)
                    .join(&unique_str_folder_name)
                    .join(&game_type_folder_name);
                SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;
                let name_prefix = unique_str_folder_name.clone();
                let output_ib_buf_file_path =
                    game_type_output_path.join(format!("{}.ib", name_prefix));
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
                    let category_output_buf_file_path = game_type_output_path
                        .join(format!("{}-{}.buf", name_prefix, category_name));
                    self.export_category_buffer(
                        category_name,
                        &category_buf_filename,
                        None,
                        d3d11_game_type.gpu_pre_skinning,
                        &category_output_buf_file_path,
                    )?;
                }
                let mut submesh_json = SubMeshJson::new();
                submesh_json.game_preset = game_preset.to_string();
                submesh_json.vertex_limit_vb = vertex_limit_vb.clone();
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
                            d3d11_element_list: self.build_workspace_elements_for_category(
                                d3d11_game_type,
                                category_name,
                                category_txt_filename_map
                                    .get(category_name)
                                    .map(String::as_str)
                                    .unwrap_or_default(),
                            )?,
                        });
                }
                submesh_json
                    .save_to_file(game_type_output_path.join(name_prefix.to_string() + ".json"))?;
            }
        }
        Ok(true)
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
        let current_gametype_folder_path = gametype_folder_path.join("APMI");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract,
            d3d11_gametype_lv2,
        })
    }

    pub fn get_possible_gametype_list_unity_vs(
        &self,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
    ) -> Result<Vec<D3D11GameType>, String> {
        let mut possible_game_type_list: Vec<D3D11GameType> = Vec::new();
        let mut find_at_least_one_gpu_type = false;

        for d3d11_game_type in self
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if !crate::extract_new::can_match_gametype(
                pointlist_index,
                d3d11_game_type.gpu_pre_skinning,
            ) {
                crate::extract_log!("未找到PointlistIndex，跳过GPU-PreSkinning数据类型");
                continue;
            }

            if find_at_least_one_gpu_type && !d3d11_game_type.gpu_pre_skinning {
                crate::extract_log!(
                    "自动优化:已经找到了满足条件的GPU类型，所以这个CPU类型就不用判断了"
                );
                continue;
            }

            crate::extract_log!("当前数据类型:{}", d3d11_game_type.game_type_name);

            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa.data,
                trianglelist_index_list,
                d3d11_game_type,
            );
            crate::extract_log!("TrianglelistIndex: {}", trianglelist_index);

            if trianglelist_index.is_empty() {
                crate::extract_log!(
                    "当前GameType无法找到符合槽位存在条件的TrianglelistIndex，跳过此项"
                );
                continue;
            }

            let mut category_buf_file_size_map: HashMap<String, u64> = HashMap::new();
            let mut all_file_exists = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let topology = d3d11_game_type
                    .category_topology_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                let mut extract_index = trianglelist_index.clone();
                if topology == "pointlist" && !pointlist_index.is_empty() {
                    extract_index = pointlist_index.to_string();
                }

                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                crate::extract_log!(
                    "当前分类:{} 提取Index: {} 提取槽位:{}",
                    category_name,
                    extract_index,
                    category_slot
                );

                let category_buf_file_name = self
                    .fa
                    .data
                    .filter_first_file(&format!("{}-{}", extract_index, category_slot), ".buf")
                    .unwrap_or_default();
                let category_buf_txt_file_name = self
                    .fa
                    .data
                    .filter_first_file(&format!("{}-{}", extract_index, category_slot), ".txt")
                    .unwrap_or_default();

                let category_buf_file_path =
                    self.fa.log.get_deduped_filepath(&category_buf_file_name);
                if category_buf_file_path.is_empty() || !Path::new(&category_buf_file_path).exists()
                {
                    crate::extract_log!("对应Buffer文件未找到,此数据类型无效。");
                    all_file_exists = false;
                    break;
                }
                let category_buf_txt_file_path = self
                    .fa
                    .log
                    .get_deduped_filepath(&category_buf_txt_file_name);
                if !category_buf_txt_file_name.is_empty() && category_buf_txt_file_path.is_empty() {
                    return Err(format!(
                        "Category {} txt file path is empty: {}",
                        category_name, category_buf_txt_file_name
                    ));
                }
                let file_size = if category_buf_txt_file_name.is_empty()
                    || category_buf_txt_file_path.is_empty()
                {
                    SSMTFileUtils::get_file_size(&category_buf_file_path)?
                } else {
                    if d3d11_game_type.gpu_pre_skinning {
                        SSMTFileUtils::get_file_size(&category_buf_file_path)?
                    } else {
                        SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_buf_txt_file_path)?
                    }
                };
                category_buf_file_size_map.insert(category_name.clone(), file_size);
            }

            if !all_file_exists {
                crate::extract_log!("当前数据类型的部分槽位文件无法找到，跳过此数据类型识别。");
                continue;
            }

            let mut vertex_number: u64 = 0;
            let mut all_match = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let category_stride = d3d11_game_type
                    .category_stride_dict
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);
                let file_size = category_buf_file_size_map
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);

                if category_stride == 0 {
                    all_match = false;
                    break;
                }

                let tmp_number = file_size / category_stride;

                if tmp_number == 0 {
                    crate::extract_log!("槽位的文件大小不能为0，槽位匹配失败，跳过此数据类型");
                    all_match = false;
                    break;
                }

                if !d3d11_game_type.gpu_pre_skinning {
                    let yu_shu = file_size % category_stride;
                    if yu_shu != 0 {
                        crate::extract_log!(
                            "余数不为0: {}，文件步长除以类别步长不能含有余数",
                            yu_shu
                        );
                        all_match = false;
                        break;
                    }

                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", trianglelist_index, category_slot),
                            ".txt",
                        )
                        .unwrap_or_default();

                    if category_txt_file_name.is_empty() {
                        crate::extract_log!("槽位的txt文件不存在，跳过此数据类型。");
                        all_match = false;
                        break;
                    }

                    let category_txt_file_path =
                        self.fa.log.get_deduped_filepath(&category_txt_file_name);
                    if category_txt_file_path.is_empty()
                        || !Path::new(&category_txt_file_path).exists()
                    {
                        all_match = false;
                        break;
                    }

                    let vertex_count_txt_show = SSMTFileUtils::find_migoto_ini_attribute_in_file(
                        &category_txt_file_path,
                        "vertex count",
                    )?;
                    let txt_show_vertex_count = vertex_count_txt_show.parse::<u64>().unwrap_or(0);

                    if txt_show_vertex_count != tmp_number {
                        crate::extract_log!(
                            "槽位的txt文件顶点数与Buffer统计顶点数不符，跳过此数据类型。"
                        );
                        all_match = false;
                        break;
                    }
                }

                if d3d11_game_type.gpu_pre_skinning && category_name == "Texcoord" {
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let expected_input_slot = category_slot
                        .strip_prefix("vb")
                        .and_then(|value| value.parse::<u64>().ok());
                    let category_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", trianglelist_index, category_slot),
                            ".txt",
                        )
                        .unwrap_or_default();
                    let category_txt_file_path =
                        self.fa.log.get_deduped_filepath(&category_txt_file_name);
                    if category_txt_file_name.is_empty()
                        || category_txt_file_path.is_empty()
                        || !Path::new(&category_txt_file_path).is_file()
                    {
                        crate::extract_log!("GPU Texcoord 布局文件不存在，跳过此数据类型。");
                        all_match = false;
                        break;
                    }
                    let vertex_buffer = VertexBufferTxtFile::new(&category_txt_file_path)?;
                    let source_stride = vertex_buffer.stride.parse::<u64>().unwrap_or(0);
                    if source_stride != category_stride {
                        crate::extract_log!(
                            "GPU Texcoord 真实步长 {} 与模板步长 {} 不一致，跳过此数据类型。",
                            source_stride,
                            category_stride
                        );
                        all_match = false;
                        break;
                    }
                    for element in &vertex_buffer.d3d11_element_list {
                        if let (Some(expected_slot), Some(actual_slot)) = (
                            expected_input_slot,
                            vertex_buffer
                                .input_slot_by_element_name
                                .get(&element.element_name),
                        ) {
                            if *actual_slot != expected_slot {
                                continue;
                            }
                        }
                        let Some(offset) = vertex_buffer
                            .aligned_byte_offset_by_element_name
                            .get(&element.element_name)
                        else {
                            crate::extract_log!(
                                "GPU Texcoord 元素 {} 缺少 AlignedByteOffset，跳过此数据类型。",
                                element.element_name
                            );
                            all_match = false;
                            break;
                        };
                        if offset.saturating_add(element.byte_width_int()) > source_stride {
                            crate::extract_log!(
                                "GPU Texcoord 元素 {} 超出真实步长，跳过此数据类型。",
                                element.element_name
                            );
                            all_match = false;
                            break;
                        }
                    }
                    if !all_match {
                        break;
                    }
                }

                if vertex_number == 0 {
                    vertex_number = tmp_number;
                } else if vertex_number != tmp_number {
                    crate::extract_log!(
                        "VertexNumber: {} 当前槽位数量: {}",
                        vertex_number,
                        tmp_number
                    );
                    crate::extract_log!("槽位匹配失败");
                    all_match = false;
                    break;
                } else {
                    crate::extract_log!("{} Match!", category_name);
                }
            }

            if all_match {
                crate::extract_log!("MatchGameType: {}", d3d11_game_type.game_type_name);
                possible_game_type_list.push(d3d11_game_type.clone());
            }

            if !find_at_least_one_gpu_type {
                for gt in possible_game_type_list.iter() {
                    if gt.gpu_pre_skinning {
                        find_at_least_one_gpu_type = true;
                        break;
                    }
                }
            }
        }

        if possible_game_type_list.is_empty() {
            crate::extract_log!("无法识别 DrawIB {} 对应的数据类型", draw_ib);
        } else {
            crate::extract_log!("All Matched GameType:");
            for d3d11_game_type in possible_game_type_list.iter() {
                crate::extract_log!("{}", d3d11_game_type.game_type_name);
            }
        }

        Ok(possible_game_type_list)
    }

    pub fn extract_38cddb8c0126726a(
        &self,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<bool, String> {
        //这条分支比较特殊，它不能直接转换为传统的.fmt .ib .vb文件
        //生成Buffer + Import_GPU_XXX.json（以数据类型为名称的Json文件）

        let mut possible_d3d11_game_type_list = self.get_possible_gametype_list_unity_vs(
            draw_ib,
            pointlist_index,
            trianglelist_index_list,
        )?;

        possible_d3d11_game_type_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));

        if possible_d3d11_game_type_list.is_empty() {
            return Ok(false);
        }

        let mut match_first_index_ib_txt_file_name_dict: BTreeMap<u64, String> = BTreeMap::new();
        for trianglelist_index in trianglelist_index_list.iter() {
            let ib_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                .unwrap_or_default();
            if ib_file_name.is_empty() {
                continue;
            }

            let ib_file_path = self.fa.log.get_deduped_filepath(&ib_file_name);
            if ib_file_path.is_empty() {
                continue;
            }

            let ib_txt_file = IndexBufferTxtFile::new(&ib_file_path, false)?;
            let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            match_first_index_ib_txt_file_name_dict.insert(match_first_index, ib_file_name);
        }

        for (match_first_index, ib_file_name) in match_first_index_ib_txt_file_name_dict.iter() {
            crate::extract_log!(
                "MatchFirstIndex: {} IBFileName: {}",
                match_first_index,
                ib_file_name
            );
        }

        for d3d11_game_type in possible_d3d11_game_type_list.iter() {
            //获取到用于提取的TrianglelistIndex
            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa.data,
                trianglelist_index_list,
                d3d11_game_type,
            );
            let vertex_limit_vb = self.build_vertex_limit_vb(&trianglelist_index);

            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);

            //输出每个Submesh，将CategoryBuffer直接复制过去，顺便生成描述文件
            for (_match_first_index, ib_txt_file_name) in
                match_first_index_ib_txt_file_name_dict.iter()
            {
                let per_ib_trianglelist_index: String = ib_txt_file_name.chars().take(6).collect();
                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                crate::extract_log!("{}", ib_buf_file_name);

                let ib_txt_file_path = self.fa.log.get_deduped_filepath(ib_txt_file_name);
                let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);

                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
                crate::extract_log!("{}", ib_txt_file_path);
                crate::extract_log!("FirstIndex: {}", ib_txt_file.first_index);
                crate::extract_log!("IndexCount: {}", ib_txt_file.index_count);

                let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
                let mut category_txt_filename_map: HashMap<String, String> = HashMap::new();
                crate::extract_log!("开始从各个Buffer文件中读取数据:");
                let mut buf_dict_list: Vec<HashMap<usize, Vec<u8>>> = Vec::new();
                for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                    let topology = d3d11_game_type
                        .category_topology_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();

                    let mut extract_index = per_ib_trianglelist_index.clone();
                    if topology == "pointlist" && !pointlist_index.is_empty() {
                        extract_index = pointlist_index.to_string();
                    }

                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let filename_search_str = format!("{}-{}", extract_index, category_slot);
                    let category_buf_filename = self
                        .fa
                        .data
                        .filter_first_file(&filename_search_str, ".buf")?;
                    let category_txt_filename = self
                        .fa
                        .data
                        .filter_first_file(&filename_search_str, ".txt")?;

                    category_buf_filename_map
                        .insert(category_name.clone(), category_buf_filename.clone());
                    category_txt_filename_map
                        .insert(category_name.clone(), category_txt_filename.clone());

                    let category_buf_file_path =
                        self.fa.log.get_deduped_filepath(&category_buf_filename);
                    let category_stride = d3d11_game_type
                        .category_stride_dict
                        .get(category_name)
                        .copied()
                        .unwrap_or(0);

                    if category_buf_file_path.is_empty() || category_stride == 0 {
                        return Err(format!(
                            "Category {} path or stride invalid: path={} stride={}",
                            category_name, category_buf_file_path, category_stride
                        ));
                    }

                    let category_buf_txt_file_path =
                        self.fa.log.get_deduped_filepath(&category_txt_filename);

                    let buf_dict;
                    if category_txt_filename.is_empty() {
                        buf_dict = SSMTBinaryUtils::read_binary_file_by_stride(
                            &category_buf_file_path,
                            category_stride as usize,
                            false,
                        )?;
                    } else {
                        let metadata = SSMTBinaryUtils::read_migoto_buffer_metadata(
                            &category_buf_txt_file_path,
                        )?;
                        if metadata.first_vertex == 0 {
                            buf_dict = SSMTBinaryUtils::read_binary_file_by_stride(
                                &category_buf_file_path,
                                category_stride as usize,
                                false,
                            )?;
                        } else {
                            buf_dict = SSMTBinaryUtils::read_binary_file_by_stride_with_offset_and_vertex_count(
								&category_buf_file_path,
								category_stride as usize,
								false,
								metadata.byte_offset,
								metadata.vertex_count,
							)?;
                        }
                    }
                    buf_dict_list.push(buf_dict);
                }

                //创建输出文件夹
                let unique_str_folder_name = format!(
                    "{}-{}-{}",
                    draw_ib, ib_txt_file.index_count, ib_txt_file.first_index
                );
                let game_type_output_path = PathBuf::from(&self.workspace_path)
                    .join(&unique_str_folder_name)
                    .join(&game_type_folder_name);
                SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;

                let name_prefix = unique_str_folder_name.clone();
                let output_ib_buf_file_path =
                    game_type_output_path.join(format!("{}.ib", name_prefix));

                //IB文件根据txt截取后输出到文件夹
                let mut ib_buf_file =
                    IndexBufferBufFile::from_file(&ib_buf_file_path, &ib_txt_file.format)?;
                ib_buf_file.self_divide(
                    ib_txt_file.first_index.parse::<usize>().unwrap_or(0),
                    ib_txt_file.index_number_count as usize,
                );
                ib_buf_file.save_to_file_uint32(&output_ib_buf_file_path, 0)?;

                //复制每个Category的Buffer文件
                for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                    let category_buf_filename = category_buf_filename_map
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_txt_filename = category_txt_filename_map
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_output_buf_file_path = game_type_output_path
                        .join(format!("{}-{}.buf", name_prefix, category_name));
                    self.export_category_buffer(
                        category_name,
                        &category_buf_filename,
                        Some(&category_txt_filename),
                        d3d11_game_type.gpu_pre_skinning,
                        &category_output_buf_file_path,
                    )?;
                }

                //如果cs-t1存在，则复制cs-t1的Buffer文件为 -Blend文件
                let cs_t1_search_str = format!("{}-{}=", &pointlist_index, "cs-t1");
                let cs_t1_buf_file_name =
                    self.fa.data.filter_first_file(&cs_t1_search_str, ".buf")?;
                let cs_t1_buf_file_path = self.fa.log.get_deduped_filepath(&cs_t1_buf_file_name);

                if cs_t1_buf_file_path.is_empty() {
                    crate::extract_log!(
                        "cs-t1 Buffer文件不存在，跳过复制cs-t1为-Blend的Buffer文件"
                    );
                } else {
                    let blend_buf_file_path =
                        game_type_output_path.join(format!("{}-Blend.buf", name_prefix));
                    fs::copy(&cs_t1_buf_file_path, &blend_buf_file_path).map_err(|e| {
                        format!(
                            "Failed to copy cs-t1 buffer file to Blend buffer file: {}",
                            e
                        )
                    })?;
                    crate::extract_log!("复制了cs-t1 Buffer文件为Blend Buffer文件");
                    crate::extract_log!(
                        "原文件: {} 目标文件: {}",
                        cs_t1_buf_file_path,
                        blend_buf_file_path.display()
                    );
                }

                //TODO 生成SubMesh.json
                let mut submesh_json = SubMeshJson::new();
                submesh_json.game_preset = "APMI".to_string();
                submesh_json.vertex_limit_vb = vertex_limit_vb.clone();
                submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
                submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
                //这里固定R32_UINT因为我们写出的ib文件就是R32_UINT
                submesh_json.index_buffer_list.push(SubMeshIndexBuffer {
                    dxgi_format: "DXGI_FORMAT_R32_UINT".to_string(),
                    file_name: format!("{}.ib", name_prefix),
                });

                for category_name in d3d11_game_type.ordered_category_name_list.iter() {
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
                            d3d11_element_list: self.build_workspace_elements_for_category(
                                d3d11_game_type,
                                category_name,
                                category_txt_filename_map
                                    .get(category_name)
                                    .map(String::as_str)
                                    .unwrap_or_default(),
                            )?,
                        });
                }

                //这里给Blend声明为DynamicBlend，在Blender端进行特殊处理
                if !cs_t1_buf_file_path.is_empty() {
                    let blend_hash = self.build_slot_hash_from_buf_file_name(
                        "Blend",
                        "cs-t1",
                        &cs_t1_buf_file_name,
                    )?;
                    submesh_json
                        .category_hash_dict
                        .insert("Blend".to_string(), blend_hash);
                    submesh_json
                        .category_draw_category_map
                        .insert("Blend".to_string(), "Blend".to_string());

                    submesh_json
                        .category_buffer_list
                        .push(SubMeshCategoryBuffer {
                            file_name: format!("{}-Blend.buf", name_prefix),
                            buffer_type: "DynamicBlend".to_string(),
                            d3d11_element_list: Vec::new(),
                        });
                }

                submesh_json
                    .save_to_file(game_type_output_path.join(name_prefix.to_string() + ".json"))?;
            }
        }

        Ok(true)
    }

    pub fn extract_fee307b98a965c16(
        &self,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<bool, String> {
        let mut possible_d3d11_game_type_list = self.get_possible_gametype_list_unity_vs(
            draw_ib,
            pointlist_index,
            trianglelist_index_list,
        )?;

        possible_d3d11_game_type_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));

        self.export_unity_vs_submeshes(
            "APMI",
            draw_ib,
            pointlist_index,
            trianglelist_index_list,
            &possible_d3d11_game_type_list,
        )
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        crate::extract_log!("开始提取:");
        let mut extracted_draw_ib_set = BTreeSet::new();
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

            let pointlist_index = self
                .fa
                .log
                .get_last_pointlist_index_by_hash(&draw_ib)
                .unwrap_or_default();
            crate::extract_log!("当前识别到的PointlistIndex: {}", pointlist_index);
            if pointlist_index.is_empty() {
                crate::extract_log!(
                    "当前识别到的PointlistIndex为空，此DrawIB可能为CPU-PreSkinning类型。"
                );
            }

            //根据DrawIB获取对应的TrianglelistIndex列表
            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(&draw_ib);

            //输出展示一下
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TrianglelistIndex: {}", trianglelist_index);
            }

            //这里APMI和异人之下差不多，都需要特殊的Blend处理，所以需要逐个识别ComputeShader的值进行处理
            //就像SRMI一样，需要先获取cs的值
            //拿到cs-cb0的文件名，判断是否包含指定的Hash
            let cs_cb0_key: String = format!("{}-cs-cb0=", pointlist_index);
            crate::extract_log!("cs-cb0 key: {}", cs_cb0_key);
            let cs_cb0_filename: String = match self.fa.data.filter_first_file(&cs_cb0_key, ".buf")
            {
                Ok(filename) => filename,
                Err(error) => {
                    crate::extract_new::handle_drawib_extraction_error(
                        draw_ib,
                        error,
                        !self.specify_drawib_extract,
                    )?;
                    continue;
                }
            };
            crate::extract_log!("cs-cb0 filename: {}", cs_cb0_filename);
            crate::extract_log!();

            if cs_cb0_filename.contains("38cddb8c0126726a") {
                let result = crate::extract_new::evaluate_drawib_extraction(
                    draw_ib,
                    self.extract_38cddb8c0126726a(
                        &draw_ib,
                        &pointlist_index,
                        &trianglelist_index_list,
                        data_type_filter,
                    ),
                    || {
                        format!(
                            "no valid data type matched. PointlistIndex: {} TrianglelistIndex: {:?}",
                            pointlist_index, trianglelist_index_list
                        )
                    },
                    !self.specify_drawib_extract,
                )?;
                if !result {
                    continue;
                }
            } else {
                let result = crate::extract_new::evaluate_drawib_extraction(
                    draw_ib,
                    self.extract_fee307b98a965c16(
                        &draw_ib,
                        &pointlist_index,
                        &trianglelist_index_list,
                        data_type_filter,
                    ),
                    || {
                        format!(
                            "no valid data type matched. PointlistIndex: {} TrianglelistIndex: {:?}",
                            pointlist_index, trianglelist_index_list
                        )
                    },
                    !self.specify_drawib_extract,
                )?;
                if !result {
                    continue;
                }
            }
            extracted_draw_ib_set.insert(draw_ib.clone());
        }

        self.write_workspace_apmi_capture_snapshot(&extracted_draw_ib_set)?;
        crate::extract_log!("提取正常执行完成");
        if self.specify_drawib_extract {
            sync_workspace_deduped_textures_and_json(
                &self.fa,
                &self.drawib_config,
                &self.workspace_path,
                !self.specify_drawib_extract,
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
            sync_workspace_deduped_textures_and_json(
                &self.fa,
                &full_drawib_config,
                &self.workspace_path,
                !self.specify_drawib_extract,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::d3d11_element::D3D11Element;

    #[test]
    fn workspace_texcoord_layout_uses_source_offsets_and_aliases() {
        let template = D3D11Element::new(
            "TEXCOORD",
            "R32G32_FLOAT",
            "vb1",
            "trianglelist",
            "Texcoord",
            "Texcoord",
            "8",
        );
        let game_type = D3D11GameType::from_parts("GPU_TEST", vec![template]);

        let mut vertex_buffer = VertexBufferTxtFile {
            stride: "20".to_string(),
            topology: "trianglelist".to_string(),
            ..VertexBufferTxtFile::default()
        };
        for (name, index, format, width, offset) in [
            ("COLOR", 0, "R8G8B8A8_UNORM", "4", 0),
            ("TEXCOORD", 0, "R32G32_FLOAT", "8", 4),
            ("TEXCOORD", 1, "R32G32_FLOAT", "8", 12),
            ("TEXCOORD", 2, "R32G32_FLOAT", "8", 12),
        ] {
            let mut element = D3D11Element::new(
                name,
                format,
                "vb1",
                "trianglelist",
                "Texcoord",
                "Texcoord",
                width,
            );
            element.semantic_index = index;
            element.element_name = if index == 0 {
                name.to_string()
            } else {
                format!("{name}{index}")
            };
            vertex_buffer
                .aligned_byte_offset_by_element_name
                .insert(element.element_name.clone(), offset);
            vertex_buffer
                .input_slot_by_element_name
                .insert(element.element_name.clone(), 1);
            vertex_buffer.d3d11_element_list.push(element);
        }
        let mut normal = D3D11Element::new(
            "NORMAL",
            "R32G32B32_FLOAT",
            "vb0",
            "trianglelist",
            "Position",
            "Position",
            "12",
        );
        normal.element_name = "NORMAL".to_string();
        vertex_buffer
            .aligned_byte_offset_by_element_name
            .insert("NORMAL".to_string(), 12);
        vertex_buffer
            .input_slot_by_element_name
            .insert("NORMAL".to_string(), 0);
        vertex_buffer.d3d11_element_list.push(normal);

        let layout = APMI2NewExtractor::build_source_texcoord_elements(
            &game_type,
            "Texcoord",
            &vertex_buffer,
        )
        .unwrap();

        assert_eq!(layout.len(), 4);
        assert_eq!(layout[1].format, "R32G32_FLOAT");
        assert_eq!(layout[1].aligned_byte_offset, "4");
        assert_eq!(layout[2].aligned_byte_offset, "12");
        assert_eq!(layout[3].aligned_byte_offset, "12");
        assert_eq!(layout[2].semantic_index, "1");
        assert_eq!(layout[3].semantic_index, "2");
        let serialized = serde_json::to_value(&layout[1]).unwrap();
        assert_eq!(serialized["AlignedByteOffset"], "4");
    }
}
