use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::d3d11_gametype_wrapper::D3D11GameTypeWrapper;
use crate::common::frame_analysis::frameanalysis_data::FrameAnalysisData;
use crate::common::frame_analysis::frameanalysis_log::FrameAnalysisSingleLog;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::config::drawib_config::DrawIBConfig;
use crate::config::path_manager::PathManager;
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::helper::mark_texture_helper::{
    get_workspace_component_name_draw_call_index_list_json_path,
    get_workspace_trianglelist_deduped_filename_json_path, ComponentNameDrawCallIndexListJson,
    TrianglelistDedupedFileNameJson, TrianglelistDedupedTextureProperty,
};
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::utils::ssmt_string_utils::SSMTStringUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshD3D11Element, SubMeshIndexBuffer, SubMeshJson,
};

//use std::fs;
//use std::path::Path;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

//SRMI提取逻辑
/*
  全新设计方案，提取器类，每个游戏都有一个对应的提取器类
  然后去初始化它的FrameAnalysisData和FrameAnalysisSingleLog对象

  这里如果是一个FrameAnalysis下有多个log.txt文件的话
  也可以魔改为fa_data_list的结构来适配
  总之每个游戏灵活应用，也能避免重复初始化的问�?
  比之前的C#里的设计更好�?

*/
pub struct SRMINewExtractor {
    fa_data: FrameAnalysisData,
    fa_log: FrameAnalysisSingleLog,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

type PrecollectedIbCandidateMap = BTreeMap<(u64, u64), Vec<String>>;

fn push_unique_string(target: &mut Vec<String>, value: &str) {
    if value.is_empty() || target.iter().any(|item| item == value) {
        return;
    }

    target.push(value.to_string());
}

impl SRMINewExtractor {
    fn add_precollected_ib_candidate(
        candidates: &mut PrecollectedIbCandidateMap,
        first_index: u64,
        index_count: u64,
        ib_txt_file_name: String,
    ) {
        let group = candidates.entry((first_index, index_count)).or_default();
        if !group.contains(&ib_txt_file_name) {
            group.push(ib_txt_file_name);
        }
    }

    fn build_index_blocks<'a>(lines: &'a [String]) -> Vec<(String, Vec<&'a str>)> {
        let mut blocks: Vec<(String, Vec<&str>)> = Vec::new();
        let mut current_index = String::new();
        let mut current_block_lines: Vec<&str> = Vec::new();

        for line in lines {
            if line.starts_with("00") && line.len() >= 6 {
                let next_index = &line[0..6];
                if current_index.is_empty() {
                    current_index = next_index.to_string();
                } else if current_index != next_index {
                    blocks.push((current_index, current_block_lines));
                    current_index = next_index.to_string();
                    current_block_lines = Vec::new();
                }
            }

            if !current_index.is_empty() {
                current_block_lines.push(line.as_str());
            }
        }

        if !current_index.is_empty() {
            blocks.push((current_index, current_block_lines));
        }

        blocks
    }

    fn extract_hash_from_log_line(line: &str) -> Option<String> {
        let start = line.find("hash=")? + 5;
        let hash: String = line[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .collect();

        if hash.is_empty() {
            return None;
        }

        Some(hash)
    }

    fn extract_vb0_hash_from_block(block_lines: &[&str]) -> Option<String> {
        let mut find_ia_set_vb = false;

        for line in block_lines {
            if line.contains("IASetVertexBuffers") && !find_ia_set_vb {
                find_ia_set_vb = true;
                continue;
            }

            if !find_ia_set_vb {
                continue;
            }

            if line.starts_with("00") {
                break;
            }

            let trimmed = line.trim_start();
            if !trimmed.starts_with("0:") {
                continue;
            }

            return Self::extract_hash_from_log_line(trimmed);
        }

        None
    }

    fn extract_copyresource_hash_pairs_from_block(block_lines: &[&str]) -> Vec<(String, String)> {
        let mut copy_pairs: Vec<(String, String)> = Vec::new();
        let mut active_src: Option<String> = None;
        let mut in_copy = false;

        for line in block_lines {
            if line.contains("CopyResource(") {
                in_copy = true;
                active_src = None;
                continue;
            }

            if !in_copy {
                continue;
            }

            let trimmed = line.trim_start();
            if trimmed.starts_with("Src:") {
                active_src = Self::extract_hash_from_log_line(trimmed);
            } else if trimmed.starts_with("Dst:") {
                if let (Some(src_hash), Some(dst_hash)) =
                    (active_src.take(), Self::extract_hash_from_log_line(trimmed))
                {
                    copy_pairs.push((src_hash, dst_hash));
                    in_copy = false;
                }
            }
        }

        copy_pairs
    }

    fn get_cs_output_vertex_limit_vb_from_lines(
        lines: &[String],
        trianglelist_index: &str,
        vertex_limit_vb: &str,
    ) -> Option<String> {
        if vertex_limit_vb.is_empty() {
            return None;
        }

        let trianglelist_index_number = trianglelist_index.parse::<i32>().ok()?;
        let blocks = Self::build_index_blocks(lines);

        for (index, block_lines) in blocks.iter().rev() {
            let index_number = match index.parse::<i32>() {
                Ok(value) => value,
                Err(_) => continue,
            };

            if index_number >= trianglelist_index_number {
                continue;
            }

            for (src_hash, dst_hash) in
                Self::extract_copyresource_hash_pairs_from_block(block_lines)
            {
                if dst_hash == vertex_limit_vb && src_hash != vertex_limit_vb {
                    return Some(src_hash);
                }
            }
        }

        None
    }

    fn get_copyresource_aware_pointlist_index_from_lines(
        lines: &[String],
        drawcall_index_list: &[String],
    ) -> Option<String> {
        if drawcall_index_list.is_empty() {
            return None;
        }

        let first_trianglelist_index = &drawcall_index_list[0];
        let trianglelist_index_number = first_trianglelist_index.parse::<i32>().ok()?;

        let blocks = Self::build_index_blocks(lines);

        let vb0_hash = blocks
            .iter()
            .find(|(index, _)| index == first_trianglelist_index)
            .and_then(|(_, block_lines)| Self::extract_vb0_hash_from_block(block_lines))?;

        let mut tracked_hashes: BTreeSet<String> = BTreeSet::new();
        tracked_hashes.insert(vb0_hash);

        let mut candidate_indices: BTreeSet<i32> = BTreeSet::new();

        for (index, block_lines) in blocks.iter().rev() {
            let index_number = match index.parse::<i32>() {
                Ok(value) => value,
                Err(_) => continue,
            };

            if index_number >= trianglelist_index_number {
                continue;
            }

            // CopyResource may appear at any position in the block, not just the first line.
            let copy_pairs = Self::extract_copyresource_hash_pairs_from_block(block_lines);

            for line in block_lines {
                let line_lowercase = line.to_ascii_lowercase();
                for hash in tracked_hashes.iter() {
                    let find_str = format!("hash={}", hash);
                    if line.contains(&find_str) && !line_lowercase.contains("dst") {
                        candidate_indices.insert(index_number);
                        break;
                    }
                }
            }

            for (src_hash, dst_hash) in copy_pairs {
                if tracked_hashes.contains(&dst_hash) {
                    tracked_hashes.insert(src_hash);
                }
            }
        }

        let result_index = *candidate_indices.iter().next_back()?;
        let result = format!("{:06}", result_index);

        // 最终校验：PointlistIndex 必须严格小于所有 TrianglelistIndex
        let all_tl_greater = drawcall_index_list.iter().all(|tl_idx| {
            tl_idx
                .parse::<i32>()
                .map(|n| result_index < n)
                .unwrap_or(true)
        });

        if all_tl_greater {
            Some(result)
        } else {
            None
        }
    }

    /// FA目录中的文件可能是 .lnk 快捷方式，无法直接读取内容和大小。
    /// 此方法优先返回 deduped 目录下对应的真实文件路径，回退时才使用 FA 目录路径。
    fn resolve_fa_filepath(&self, fa_filename: &str) -> PathBuf {
        let deduped = self.fa_log.get_deduped_filepath(fa_filename);
        if !deduped.is_empty() {
            let p = PathBuf::from(&deduped);
            if p.exists() {
                return p;
            }
        }
        self.fa_data.dir.join(fa_filename)
    }

    fn get_copyresource_aware_pointlist_index(&self, draw_ib: &str) -> String {
        let drawcall_index_list = self.fa_log.get_drawcall_index_list_by_hash(draw_ib, false);

        Self::get_copyresource_aware_pointlist_index_from_lines(
            &self.fa_log.lines,
            &drawcall_index_list,
        )
        .or_else(|| self.fa_log.get_last_pointlist_index_by_hash(draw_ib))
        .unwrap_or_default()
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

    fn normalize_dxgi_format(format: &str) -> String {
        let upper = format.trim().to_ascii_uppercase();
        upper
            .strip_prefix("DXGI_FORMAT_")
            .unwrap_or(&upper)
            .to_string()
    }

    fn read_migoto_input_layout_for_slot(
        txt_file_path: &Path,
        input_slot: u64,
    ) -> Result<Vec<(String, u64, String, u64)>, String> {
        let content = fs::read_to_string(txt_file_path).map_err(|e| {
            format!(
                "Failed to read migoto layout txt {}: {}",
                txt_file_path.display(),
                e
            )
        })?;

        let mut result: Vec<(String, u64, String, u64)> = Vec::new();

        for block in content.split("element[").skip(1) {
            let mut semantic_name: Option<String> = None;
            let mut semantic_index: Option<u64> = None;
            let mut format: Option<String> = None;
            let mut slot: Option<u64> = None;
            let mut aligned_byte_offset: Option<u64> = None;

            for line in block.lines() {
                let trimmed = line.trim();
                if trimmed.eq_ignore_ascii_case("vertex-data:") {
                    break;
                }

                if let Some(value) = trimmed.strip_prefix("SemanticName:") {
                    semantic_name = Some(value.trim().to_string());
                } else if let Some(value) = trimmed.strip_prefix("SemanticIndex:") {
                    semantic_index = value.trim().parse::<u64>().ok();
                } else if let Some(value) = trimmed.strip_prefix("Format:") {
                    format = Some(Self::normalize_dxgi_format(value));
                } else if let Some(value) = trimmed.strip_prefix("InputSlot:") {
                    slot = value.trim().parse::<u64>().ok();
                } else if let Some(value) = trimmed.strip_prefix("AlignedByteOffset:") {
                    aligned_byte_offset = value.trim().parse::<u64>().ok();
                }
            }

            if let (
                Some(semantic_name),
                Some(semantic_index),
                Some(format),
                Some(slot),
                Some(aligned_byte_offset),
            ) = (
                semantic_name,
                semantic_index,
                format,
                slot,
                aligned_byte_offset,
            ) {
                if slot == input_slot {
                    result.push((semantic_name, semantic_index, format, aligned_byte_offset));
                }
            }
        }

        Ok(result)
    }

    fn category_matches_migoto_input_layout(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_name: &str,
        category_slot: &str,
        txt_file_path: &Path,
    ) -> Result<bool, String> {
        let slot_lower = category_slot.trim().to_ascii_lowercase();
        let Some(input_slot) = slot_lower
            .strip_prefix("vb")
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return Ok(true);
        };

        let actual_elements = Self::read_migoto_input_layout_for_slot(txt_file_path, input_slot)?;
        if actual_elements.is_empty() {
            return Ok(true);
        }

        let mut expected_offset = 0u64;
        for element_name in &d3d11_game_type.ordered_full_element_list {
            let Some(element) = d3d11_game_type
                .element_name_d3d11_element_dict
                .get(element_name)
            else {
                continue;
            };
            if element.category != category_name {
                continue;
            }

            let expected_format = Self::normalize_dxgi_format(&element.format);
            let found = actual_elements.iter().any(
                |(semantic_name, semantic_index, format, aligned_byte_offset)| {
                    semantic_name.eq_ignore_ascii_case(&element.semantic_name)
                        && *semantic_index == element.semantic_index
                        && format.eq_ignore_ascii_case(&expected_format)
                        && *aligned_byte_offset == expected_offset
                },
            );

            if !found {
                crate::extract_log!(
                    "InputLayout mismatch: GameType={}, category={}, slot={}, expected {}{} {} at byte offset {}",
                    d3d11_game_type.game_type_name,
                    category_name,
                    category_slot,
                    element.semantic_name,
                    element.semantic_index,
                    expected_format,
                    expected_offset
                );
                return Ok(false);
            }

            expected_offset = expected_offset
                .checked_add(element.byte_width_int())
                .ok_or_else(|| {
                    format!(
                        "InputLayout expected offset overflow: GameType={} category={}",
                        d3d11_game_type.game_type_name, category_name
                    )
                })?;
        }

        Ok(true)
    }

    fn export_category_buffer(
        &self,
        category_name: &str,
        category_buf_filename: &str,
        gpu_pre_skinning: bool,
        gpu_vertex_slice: Option<(usize, usize, usize)>,
        category_output_buf_file_path: &Path,
    ) -> Result<(), String> {
        crate::extract_log!(
            "[export_category_buffer] start: category_name={}, category_buf_filename={}, category_output_buf_file_path={}",
            category_name,
            category_buf_filename,
            category_output_buf_file_path.display()
        );

        let category_buf_file_path = self.fa_log.get_deduped_filepath(category_buf_filename);
        crate::extract_log!(
            "[export_category_buffer] resolved deduped buf path: category_name={}, category_buf_file_path={}",
            category_name,
            category_buf_file_path
        );
        if category_buf_file_path.is_empty() {
            crate::extract_log!(
                "[export_category_buffer] branch=error_empty_buf_path: category_name={}, category_buf_filename={}, category_buf_file_path_is_empty=true",
                category_name,
                category_buf_filename
            );
            return Err(format!(
                "Category {} deduped path is empty: {}",
                category_name, category_buf_filename
            ));
        }

        if gpu_pre_skinning {
            if let Some((byte_offset, vertex_count, stride)) = gpu_vertex_slice {
                let category_buf_bytes = fs::read(&category_buf_file_path).map_err(|e| {
                    format!(
                        "Failed to read GPU category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                let read_len = vertex_count.checked_mul(stride).ok_or_else(|| {
                    format!(
                        "GPU category {} slice length overflow: vertex_count={} stride={}",
                        category_name, vertex_count, stride
                    )
                })?;
                let end = byte_offset.checked_add(read_len).ok_or_else(|| {
                    format!(
                        "GPU category {} slice end overflow: byte_offset={} read_len={}",
                        category_name, byte_offset, read_len
                    )
                })?;
                let sliced_bytes =
                    SSMTBinaryUtils::get_range_bytes(&category_buf_bytes, byte_offset, end)
                        .map_err(|e| {
                            format!(
                                "Failed to slice GPU category buffer for category {}: {}",
                                category_name, e
                            )
                        })?;
                fs::write(category_output_buf_file_path, sliced_bytes).map_err(|e| {
                    format!(
                        "Failed to write sliced GPU category buffer for category {}: {}",
                        category_name, e
                    )
                })?;
                crate::extract_log!(
                    "[export_category_buffer] branch=gpu_logical_slice: category_name={}, byte_offset={}, vertex_count={}, stride={}, output_size={}",
                    category_name,
                    byte_offset,
                    vertex_count,
                    stride,
                    read_len
                );
                return Ok(());
            }

            crate::extract_log!(
                "[export_category_buffer] branch=gpu_full_copy: category_name={}, gpu_pre_skinning=true, logical_slice_unavailable=true",
                category_name
            );
            fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
                format!(
                    "Failed to copy category buffer file for category {}: {}",
                    category_name, e
                )
            })?;
            return Ok(());
        }

        let category_txt_filename =
            SSMTFileUtils::get_filename_with_new_extension(category_buf_filename, "txt")?;
        let category_txt_file_path = self.fa_log.get_deduped_filepath(&category_txt_filename);
        crate::extract_log!(
            "[export_category_buffer] resolved txt path: category_name={}, category_txt_filename={}, category_txt_file_path={}",
            category_name,
            category_txt_filename,
            category_txt_file_path
        );

        if !category_txt_file_path.is_empty() && Path::new(&category_txt_file_path).exists() {
            crate::extract_log!(
                "[export_category_buffer] branch=txt_metadata_slice: category_name={}, category_txt_file_exists=true",
                category_name
            );
            let metadata = SSMTBinaryUtils::read_migoto_buffer_metadata(&category_txt_file_path)?;
            crate::extract_log!(
                "[export_category_buffer] metadata: category_name={}, stride={}, vertex_count={}, byte_offset={}",
                category_name,
                metadata.stride,
                metadata.vertex_count,
                metadata.byte_offset
            );
            if metadata.stride > 0 && metadata.vertex_count > 0 {
                crate::extract_log!(
                    "[export_category_buffer] branch=metadata_valid_slice: category_name={}, stride_valid=true, vertex_count_valid=true",
                    category_name
                );
                let category_buf_bytes = fs::read(&category_buf_file_path).map_err(|e| {
                    format!(
                        "Failed to read category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                crate::extract_log!(
                    "[export_category_buffer] read buf bytes: category_name={}, category_buf_bytes_len={}",
                    category_name,
                    category_buf_bytes.len()
                );
                let read_len = metadata
                    .vertex_count
                    .checked_mul(metadata.stride)
                    .ok_or_else(|| {
                        format!(
                            "Category {} slice length overflow: vertex_count={} stride={}",
                            category_name, metadata.vertex_count, metadata.stride
                        )
                    })?;
                crate::extract_log!(
                    "[export_category_buffer] computed read_len: category_name={}, read_len={}",
                    category_name,
                    read_len
                );
                let end = metadata.byte_offset.checked_add(read_len).ok_or_else(|| {
                    format!(
                        "Category {} slice end overflow: byte_offset={} read_len={}",
                        category_name, metadata.byte_offset, read_len
                    )
                })?;
                crate::extract_log!(
                    "[export_category_buffer] computed slice range: category_name={}, byte_offset={}, end={}",
                    category_name,
                    metadata.byte_offset,
                    end
                );
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
                crate::extract_log!(
                    "[export_category_buffer] sliced bytes: category_name={}, sliced_bytes_len={}",
                    category_name,
                    sliced_bytes.len()
                );
                fs::write(category_output_buf_file_path, sliced_bytes).map_err(|e| {
                    format!(
                        "Failed to write category buffer file for category {}: {}",
                        category_name, e
                    )
                })?;
                crate::extract_log!(
                    "[export_category_buffer] branch=write_sliced_success: category_name={}, output_path={}",
                    category_name,
                    category_output_buf_file_path.display()
                );
                return Ok(());
            }

            crate::extract_log!(
                "[export_category_buffer] branch=metadata_invalid_fallback_copy: category_name={}, stride={}, vertex_count={}",
                category_name,
                metadata.stride,
                metadata.vertex_count
            );
        }

        if category_txt_file_path.is_empty() {
            crate::extract_log!(
                "[export_category_buffer] branch=txt_missing_fallback_copy: category_name={}, category_txt_filename={}, category_txt_file_path_is_empty=true",
                category_name,
                category_txt_filename
            );
        } else if !Path::new(&category_txt_file_path).exists() {
            crate::extract_log!(
                "[export_category_buffer] branch=txt_not_found_fallback_copy: category_name={}, category_txt_filename={}, category_txt_file_path={}",
                category_name,
                category_txt_filename,
                category_txt_file_path
            );
        }

        fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
            format!(
                "Failed to copy category buffer file for category {}: {}",
                category_name, e
            )
        })?;
        crate::extract_log!(
            "[export_category_buffer] branch=copy_success: category_name={}, source_path={}, output_path={}",
            category_name,
            category_buf_file_path,
            category_output_buf_file_path.display()
        );
        Ok(())
    }

    fn validate_precollected_candidate(
        &self,
        ib_txt_file_name: &str,
        pointlist_index: &str,
        d3d11_game_type: &D3D11GameType,
        d3d11_gametype_wrapper: &D3D11GameTypeWrapper,
    ) -> Result<(HashMap<String, String>, Vec<String>), String> {
        let trianglelist_index: String = ib_txt_file_name.chars().take(6).collect();
        let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
        let mut failures: Vec<String> = Vec::new();

        let ib_buf_file_name =
            SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
        for (kind, file_name) in [
            ("txt", ib_txt_file_name),
            ("buf", ib_buf_file_name.as_str()),
        ] {
            let file_path = self.fa_log.get_deduped_filepath(file_name);
            if file_path.is_empty() || !Path::new(&file_path).exists() {
                failures.push(format!("IndexBuffer/ib ({} dump missing)", kind));
            }
        }

        for category_name in &d3d11_game_type.ordered_category_name_list {
            let topology = d3d11_game_type
                .category_topology_dict
                .get(category_name)
                .cloned()
                .unwrap_or_default();

            let mut extract_index = trianglelist_index.clone();
            if topology == "pointlist" {
                if pointlist_index.is_empty() {
                    failures.push(format!(
                        "Category={} (pointlist index missing)",
                        category_name
                    ));
                    continue;
                }

                extract_index = pointlist_index.to_string();
                if category_name == "Position" {
                    extract_index = d3d11_gametype_wrapper.position_extract_index.clone();
                } else if category_name == "Blend" {
                    extract_index = d3d11_gametype_wrapper.blend_extract_index.clone();
                }
            }

            let mut category_slot = d3d11_game_type
                .category_slot_dict
                .get(category_name)
                .cloned()
                .unwrap_or_default();
            if category_name == "Position" {
                category_slot = d3d11_gametype_wrapper.position_extract_slot.clone();
            } else if category_name == "Blend" {
                category_slot = d3d11_gametype_wrapper.blend_extract_slot.clone();
            }

            if extract_index.is_empty() || category_slot.is_empty() {
                failures.push(format!(
                    "Category={} VB={} (extract index or slot missing)",
                    category_name, category_slot
                ));
                continue;
            }

            let search_key = format!("{}-{}=", extract_index, category_slot);
            let category_buf_file_name = self.fa_data.filter_first_file(&search_key, ".buf")?;
            let category_buf_file_path = self.fa_log.get_deduped_filepath(&category_buf_file_name);
            if category_buf_file_name.is_empty()
                || category_buf_file_path.is_empty()
                || !Path::new(&category_buf_file_path).exists()
            {
                failures.push(format!(
                    "Category={} VB={} (buffer dump missing)",
                    category_name, category_slot
                ));
                continue;
            }

            // InputLayout matching belongs to GameType discovery. Do not apply it again
            // here: specialized extraction intentionally exports every stride-compatible
            // GameType interpretation, even when the active shader uses another format.
            // Candidate selection only needs to guarantee that every required Category/VB
            // dump for the already selected GameType actually exists.
            category_buf_filename_map.insert(category_name.clone(), category_buf_file_name);
        }

        Ok((category_buf_filename_map, failures))
    }

    fn select_precollected_candidate(
        &self,
        first_index: u64,
        index_count: u64,
        ib_txt_file_names: &[String],
        pointlist_index: &str,
        d3d11_game_type: &D3D11GameType,
        d3d11_gametype_wrapper: &D3D11GameTypeWrapper,
    ) -> Result<(String, HashMap<String, String>), String> {
        let mut attempt_details: Vec<String> = Vec::new();

        for ib_txt_file_name in ib_txt_file_names {
            let draw_call: String = ib_txt_file_name.chars().take(6).collect();
            let (category_buf_filename_map, failures) = self.validate_precollected_candidate(
                ib_txt_file_name,
                pointlist_index,
                d3d11_game_type,
                d3d11_gametype_wrapper,
            )?;

            if failures.is_empty() {
                crate::extract_log!(
                    "Selected DrawCall candidate: FirstIndex={}, IndexCount={}, DrawCall={}",
                    first_index,
                    index_count,
                    draw_call
                );
                return Ok((ib_txt_file_name.clone(), category_buf_filename_map));
            }

            let failure_detail = failures.join(", ");
            crate::extract_log!(
                "Rejected DrawCall candidate: FirstIndex={}, IndexCount={}, DrawCall={}, Missing={}",
                first_index,
                index_count,
                draw_call,
                failure_detail
            );
            attempt_details.push(format!("DrawCall {}: {}", draw_call, failure_detail));
        }

        let error = format!(
            "No DrawCall candidate satisfies GameType {} for (FirstIndex={}, IndexCount={}). Attempts: {}",
            d3d11_game_type.game_type_name,
            first_index,
            index_count,
            attempt_details.join("; ")
        );
        crate::extract_log!("{}", error);
        Err(error)
    }

    fn export_precollected_submeshes(
        &self,
        game_preset: &str,
        draw_ib: &str,
        vertex_limit_vb: &str,
        cs_output_vertex_limit_vb: &str,
        pointlist_index: &str,
        d3d11_game_type: &D3D11GameType,
        d3d11_gametype_wrapper: &D3D11GameTypeWrapper,
        precollected_ib_candidates: &PrecollectedIbCandidateMap,
    ) -> Result<bool, String> {
        if precollected_ib_candidates.is_empty() {
            return Ok(false);
        }
        for ((first_index, index_count), ib_txt_file_names) in precollected_ib_candidates {
            let (ib_txt_file_name, category_buf_filename_map) = self
                .select_precollected_candidate(
                    *first_index,
                    *index_count,
                    ib_txt_file_names,
                    pointlist_index,
                    d3d11_game_type,
                    d3d11_gametype_wrapper,
                )?;
            let ib_buf_file_name =
                SSMTFileUtils::get_filename_with_new_extension(&ib_txt_file_name, "buf")?;
            let ib_txt_file_path = self.fa_log.get_deduped_filepath(&ib_txt_file_name);
            let ib_buf_file_path = self.fa_log.get_deduped_filepath(&ib_buf_file_name);
            if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
                return Err(format!(
                    "Selected DrawCall {} lost its deduped index buffer files",
                    ib_txt_file_name.chars().take(6).collect::<String>()
                ));
            }
            let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;

            // GPU-pre-skinning vertex streams are a shared global vertex domain.
            // A draw-time vb0 "vertex count" is only the highest addressed vertex
            // plus one for that draw; it is NOT the size of this submesh's Position
            // stream. Derive the actual global domain from the other matched vertex
            // categories (normally Texcoord and Blend), whose full buffers are
            // already exported unchanged, then trim the oversized Position backing
            // resource to exactly that same global vertex count.
            let gpu_position_slice: Option<(usize, usize, usize)> = if d3d11_game_type
                .gpu_pre_skinning
            {
                let mut logical_vertex_count: Option<u64> = None;
                let mut logical_domain_valid = true;

                for logical_category_name in d3d11_game_type.ordered_category_name_list.iter() {
                    if logical_category_name == "Position" {
                        continue;
                    }

                    let logical_stride = d3d11_game_type
                        .category_stride_dict
                        .get(logical_category_name)
                        .copied()
                        .unwrap_or(0);
                    if logical_stride == 0 {
                        continue;
                    }

                    let logical_buf_filename = category_buf_filename_map
                        .get(logical_category_name)
                        .cloned()
                        .unwrap_or_default();
                    if logical_buf_filename.is_empty() {
                        continue;
                    }

                    let logical_buf_path = self.fa_log.get_deduped_filepath(&logical_buf_filename);
                    if logical_buf_path.is_empty() || !Path::new(&logical_buf_path).exists() {
                        continue;
                    }

                    let logical_buf_size = SSMTFileUtils::get_file_size(&logical_buf_path)?;
                    if logical_buf_size == 0 || logical_buf_size % logical_stride != 0 {
                        crate::extract_log!(
                                "GPU global vertex domain unavailable: category={}, filesize={}, stride={}",
                                logical_category_name,
                                logical_buf_size,
                                logical_stride
                            );
                        logical_domain_valid = false;
                        break;
                    }

                    let category_vertex_count = logical_buf_size / logical_stride;
                    if category_vertex_count == 0 {
                        logical_domain_valid = false;
                        break;
                    }

                    if let Some(previous_vertex_count) = logical_vertex_count {
                        if previous_vertex_count != category_vertex_count {
                            crate::extract_log!(
                                    "GPU global vertex domain mismatch: category={} has {} vertices, previous categories have {}",
                                    logical_category_name,
                                    category_vertex_count,
                                    previous_vertex_count
                                );
                            logical_domain_valid = false;
                            break;
                        }
                    } else {
                        logical_vertex_count = Some(category_vertex_count);
                    }
                }

                if logical_domain_valid {
                    if let Some(vertex_count) = logical_vertex_count {
                        let position_stride = d3d11_game_type
                            .category_stride_dict
                            .get("Position")
                            .copied()
                            .unwrap_or(0);
                        let position_buf_filename = category_buf_filename_map
                            .get("Position")
                            .cloned()
                            .unwrap_or_default();
                        let position_buf_path =
                            self.fa_log.get_deduped_filepath(&position_buf_filename);

                        if position_stride == 0
                            || position_buf_path.is_empty()
                            || !Path::new(&position_buf_path).exists()
                        {
                            None
                        } else {
                            let position_buf_size =
                                SSMTFileUtils::get_file_size(&position_buf_path)?;
                            let position_capacity = position_buf_size / position_stride;

                            if position_capacity < vertex_count {
                                crate::extract_log!(
                                        "GPU Position global-domain slice unavailable: position_capacity={} < logical_vertex_count={}",
                                        position_capacity,
                                        vertex_count
                                    );
                                None
                            } else {
                                crate::extract_log!(
                                        "GPU Position global-domain slice: logical_vertex_count={}, position_capacity={}, position_stride={}",
                                        vertex_count,
                                        position_capacity,
                                        position_stride
                                    );
                                Some((0, vertex_count as usize, position_stride as usize))
                            }
                        }
                    } else {
                        crate::extract_log!(
                                "GPU Position global-domain slice unavailable: no non-Position category supplied a logical vertex count"
                            );
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

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

                crate::extract_log!(
                    "category_name: {}, category_buf_filename: {}",
                    category_name,
                    category_buf_filename
                );
                let category_output_buf_file_path =
                    game_type_output_path.join(format!("{}-{}.buf", name_prefix, category_name));

                crate::extract_log!(
                    "category_name: {}, category_output_buf_file_path: {}",
                    category_name,
                    category_output_buf_file_path.display()
                );
                let gpu_vertex_slice =
                    if d3d11_game_type.gpu_pre_skinning && category_name == "Position" {
                        gpu_position_slice
                    } else {
                        None
                    };
                self.export_category_buffer(
                    category_name,
                    &category_buf_filename,
                    d3d11_game_type.gpu_pre_skinning,
                    gpu_vertex_slice,
                    &category_output_buf_file_path,
                )?;
            }
            let mut submesh_json = SubMeshJson::new();
            submesh_json.game_preset = game_preset.to_string();
            submesh_json.vertex_limit_vb = vertex_limit_vb.to_string();
            submesh_json.cs_output_vertex_limit_vb = cs_output_vertex_limit_vb.to_string();
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
            submesh_json
                .save_to_file(game_type_output_path.join(name_prefix.to_string() + ".json"))?;
        }
        Ok(true)
    }

    fn sync_workspace_deduped_textures_and_json(
        &self,
        draw_ib_list: &[String],
    ) -> Result<(), String> {
        let mut component_drawcall_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();

        for draw_ib in draw_ib_list.iter() {
            let draw_ib = draw_ib.trim();
            if draw_ib.is_empty() {
                continue;
            }

            let trianglelist_index_list = self.fa_data.get_trianglelist_index_list(&draw_ib);
            if trianglelist_index_list.is_empty() {
                continue;
            }

            let mut first_index_ib_txt_map: BTreeMap<u64, String> = BTreeMap::new();
            let mut first_index_index_count_map: BTreeMap<u64, String> = BTreeMap::new();
            let mut first_index_trianglelist_index_map: BTreeMap<u64, Vec<String>> =
                BTreeMap::new();

            for trianglelist_index in trianglelist_index_list.iter() {
                let search_key = format!("{}-ib", trianglelist_index);
                let ib_txt_filename = self
                    .fa_data
                    .filter_first_file(&search_key, ".txt")
                    .unwrap_or_default();
                if ib_txt_filename.is_empty() {
                    continue;
                }

                let ib_txt_filepath = self.fa_log.get_deduped_filepath(&ib_txt_filename);
                if ib_txt_filepath.is_empty() || !Path::new(&ib_txt_filepath).exists() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(ib_txt_filepath, false)?;
                let first_index = ib_txt_file.first_index.trim().parse::<u64>().unwrap_or(0);
                first_index_ib_txt_map.insert(first_index, ib_txt_filename);
                first_index_index_count_map
                    .insert(first_index, ib_txt_file.index_count.trim().to_string());
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

            for (first_index, submesh_trianglelist_index_list) in first_index_trianglelist_index_map
            {
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

        let deduped_folder_path = PathBuf::from(&self.workspace_path).join("DedupedTextures");
        SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path)?;

        for index_list in component_drawcall_index_list_dict.values() {
            for trianglelist_index in index_list.iter() {
                let content_str = format!("{}-ps-t", trianglelist_index);
                let ps_texture_all_filename_list =
                    self.fa_data.filter_texture_filename_list(&content_str);

                for ps_texture_filename in ps_texture_all_filename_list {
                    let deduped_filepath = self.fa_log.get_deduped_filepath(&ps_texture_filename);
                    if deduped_filepath.is_empty() {
                        continue;
                    }

                    let deduped_filename = self.fa_log.get_deduped_filename(&ps_texture_filename);
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
            get_workspace_component_name_draw_call_index_list_json_path(&self.workspace_path)?;
        let component_json = ComponentNameDrawCallIndexListJson::from_map(
            component_drawcall_index_list_dict.clone(),
        );
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
                let ps_texture_all_filename_list =
                    self.fa_data.filter_texture_filename_list(&content_str);
                trianglelist_texture_file_name_list.extend(ps_texture_all_filename_list);
            }
        }

        let mut trianglelist_deduped_map: HashMap<String, TrianglelistDedupedTextureProperty> =
            HashMap::new();
        for trianglelist_texture_file_name in trianglelist_texture_file_name_list {
            let hash =
                SSMTStringUtils::get_file_hash_from_file_name(&trianglelist_texture_file_name);

            let fa_log_deduped_filename = {
                let deduped = self
                    .fa_log
                    .get_deduped_filename(&trianglelist_texture_file_name);
                if deduped.trim().is_empty() {
                    String::new()
                } else {
                    format!("{}_{}", hash, deduped)
                }
            };

            let fa_data_deduped_filename = {
                let mut out = String::new();
                let deduped_dir = Path::new(&self.fa_data.dir).join("deduped");
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
            get_workspace_trianglelist_deduped_filename_json_path(&self.workspace_path)?;
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

    pub fn new(
        frame_analysis_folder: String,
        workspace_path: String,
        is_full_extract: bool,
    ) -> Result<Self, String> {
        Self::new_internal(frame_analysis_folder, workspace_path, !is_full_extract)
    }

    fn new_internal(
        frame_analysis_folder: String,
        workspace_path: String,
        specify_drawib_extract: bool,
    ) -> Result<Self, String> {
        //确保FrameAnalysis文件夹存�?
        let frame_analysis_dir = PathBuf::from(&frame_analysis_folder);
        if !frame_analysis_dir.exists() {
            return Err(format!(
                "FrameAnalysis 文件夹未找到: {}",
                frame_analysis_folder
            ));
        }

        //初始化FrameAnalyis数据对象
        let fa_data = FrameAnalysisData::new(&frame_analysis_folder)
            .map_err(|e| format!("Failed to read FrameAnalysis folder: {}", e))?;

        let fa_log = FrameAnalysisSingleLog::new(&frame_analysis_folder)
            .map_err(|e| format!("Failed to read FrameAnalysis log: {}", e))?;

        //在工作空间中读取DrawIB列表
        let drawib_config = if specify_drawib_extract {
            DrawIBConfig::new_from_workspace(&workspace_path)
                .map_err(|e| format!("Failed to read DrawIB config: {}", e))?
        } else {
            DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            }
        };

        //拼接出数据类型文件夹
        let gametype_folder_path = PathManager::bundled_gametype_folder();
        let current_gametype_folder_path = gametype_folder_path.join("SRMI");

        //创建一个D3D11GameTypeLv2对象
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa_data,
            fa_log,
            workspace_path,
            drawib_config,
            specify_drawib_extract,
            d3d11_gametype_lv2,
        })
    }

    fn parse_index_from_filename(file_name: &str) -> Option<i32> {
        if file_name.len() < 6 {
            return None;
        }
        file_name.get(0..6)?.parse::<i32>().ok()
    }

    fn extract_slot_from_filename(file_name: &str) -> String {
        let Some(start_pos) = file_name.find('-') else {
            return String::new();
        };
        let Some(end_pos) = file_name.find('=') else {
            return String::new();
        };
        if end_pos <= start_pos + 1 {
            return String::new();
        }
        file_name[start_pos + 1..end_pos].to_string()
    }

    fn is_blend_slot_match(
        &self,
        pointlist_index: &str,
        blend_stride: u64,
        vertex_count: u64,
        blend_slot: &str,
    ) -> Result<bool, String> {
        if pointlist_index.is_empty() || blend_stride == 0 {
            return Ok(false);
        }

        let search_key = format!("{}-{}=", pointlist_index, blend_slot);
        let blend_buffer_file_name = self
            .fa_data
            .filter_first_file(&search_key, ".buf")
            .unwrap_or_default();

        if blend_buffer_file_name.is_empty() {
            return Ok(false);
        }

        let blend_buffer_file_path = self.fa_log.get_deduped_filepath(&blend_buffer_file_name);
        if blend_buffer_file_path.is_empty() || !Path::new(&blend_buffer_file_path).exists() {
            return Ok(false);
        }

        let blend_buffer_size = SSMTFileUtils::get_file_size(&blend_buffer_file_path)?;
        let blend_vertex_count = blend_buffer_size / blend_stride;
        Ok(blend_vertex_count == vertex_count)
    }

    fn is_position_blend_slot_match(
        &self,
        pointlist_index: &str,
        position_stride: u64,
        blend_stride: u64,
        vertex_count: u64,
        position_slot: &str,
        blend_slot: &str,
    ) -> Result<bool, String> {
        if pointlist_index.is_empty() || position_stride == 0 || blend_stride == 0 {
            return Ok(false);
        }

        let position_key = format!("{}-{}=", pointlist_index, position_slot);
        let position_buffer_file_name = self
            .fa_data
            .filter_first_file(&position_key, ".buf")
            .unwrap_or_default();
        if position_buffer_file_name.is_empty() {
            return Ok(false);
        }

        let position_buffer_file_path =
            self.fa_log.get_deduped_filepath(&position_buffer_file_name);
        if position_buffer_file_path.is_empty() || !Path::new(&position_buffer_file_path).exists() {
            return Ok(false);
        }

        let position_size = SSMTFileUtils::get_file_size(&position_buffer_file_path)?;
        let position_vertex_count = position_size / position_stride;
        if position_vertex_count != vertex_count {
            return Ok(false);
        }

        crate::extract_log!("Position Slot Matched : {}", position_slot);

        self.is_blend_slot_match(pointlist_index, blend_stride, vertex_count, blend_slot)
    }

    fn get_pre_position_uav_index(
        &self,
        pointlist_index: &str,
        position_stride: u64,
        vertex_count: u64,
        slot: &str,
    ) -> Result<Option<(String, String)>, String> {
        if pointlist_index.is_empty() || position_stride == 0 {
            return Ok(None);
        }

        let current_key = format!("{}-{}=", pointlist_index, slot);
        let current_uav_file_name = self
            .fa_data
            .filter_first_file(&current_key, ".buf")
            .unwrap_or_default();
        if current_uav_file_name.is_empty() {
            return Ok(None);
        }

        let current_hash = SSMTStringUtils::get_file_hash_from_file_name(&current_uav_file_name);
        if current_hash.is_empty() {
            return Ok(None);
        }

        let pointlist_index_number = match pointlist_index.parse::<i32>() {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };

        let mut candidates = self
            .fa_data
            .filter_filelist(&current_hash, ".buf")
            .into_iter()
            .filter(|name| name.contains("-cs=4e03bd5b704abbdd"))
            .filter_map(|name| {
                let idx = Self::parse_index_from_filename(&name)?;
                if idx >= pointlist_index_number {
                    return None;
                }
                Some((idx, name))
            })
            .collect::<Vec<(i32, String)>>();

        if candidates.is_empty() {
            return Ok(None);
        }

        candidates.sort_by_key(|(idx, _)| *idx);
        let (_, pre_position_file_name) = candidates[candidates.len() - 1].clone();

        let pre_position_file_path = self.fa_log.get_deduped_filepath(&pre_position_file_name);
        if pre_position_file_path.is_empty() || !Path::new(&pre_position_file_path).exists() {
            return Ok(None);
        }

        let pre_position_size = SSMTFileUtils::get_file_size(&pre_position_file_path)?;
        let pre_position_vertex_count = pre_position_size / position_stride;
        if pre_position_vertex_count != vertex_count {
            return Ok(None);
        }

        let slot_name = Self::extract_slot_from_filename(&pre_position_file_name);
        let index_name = pre_position_file_name
            .get(0..6)
            .unwrap_or_default()
            .to_string();
        if slot_name.is_empty() || index_name.is_empty() {
            return Ok(None);
        }

        Ok(Some((index_name, slot_name)))
    }

    pub fn auto_gametype_detect_1c932707d4d8df41_4d9c23fd387846c7(
        &self,
        pointlist_index: String,
        trianglelist_index_list: Vec<String>,
    ) -> Result<Vec<D3D11GameTypeWrapper>, String> {
        let mut possible_list: Vec<D3D11GameTypeWrapper> = Vec::new();

        if pointlist_index.is_empty() {
            return Ok(possible_list);
        }

        for d3d11_game_type in self
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if !d3d11_game_type.gpu_pre_skinning {
                continue;
            }

            if !d3d11_game_type
                .category_slot_dict
                .values()
                .any(|v| v == "vb1")
            {
                continue;
            }

            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa_data,
                &trianglelist_index_list,
                d3d11_game_type,
            );
            if trianglelist_index.is_empty() {
                continue;
            }

            let vb1_search_key = format!("{}-vb1=", trianglelist_index);
            let vb1_buffer_filename = self
                .fa_data
                .filter_first_file(&vb1_search_key, ".buf")
                .unwrap_or_default();
            if vb1_buffer_filename.is_empty() {
                continue;
            }

            let vb1_buffer_filepath = self.fa_log.get_deduped_filepath(&vb1_buffer_filename);
            if vb1_buffer_filepath.is_empty() || !Path::new(&vb1_buffer_filepath).exists() {
                continue;
            }

            let texcoord_stride = d3d11_game_type
                .category_stride_dict
                .get("Texcoord")
                .copied()
                .unwrap_or(0);
            let position_stride = d3d11_game_type
                .category_stride_dict
                .get("Position")
                .copied()
                .unwrap_or(0);
            let blend_stride = d3d11_game_type
                .category_stride_dict
                .get("Blend")
                .copied()
                .unwrap_or(0);
            if texcoord_stride == 0 || position_stride == 0 || blend_stride == 0 {
                continue;
            }

            let vb1_size = SSMTFileUtils::get_file_size(&vb1_buffer_filepath)?;
            let vertex_count = vb1_size / texcoord_stride;

            let slot_pairs = [
                ("cs-t0", "cs-t5"),
                ("cs-t1", "cs-t6"),
                ("cs-t2", "cs-t7"),
                ("cs-t3", "cs-t8"),
                ("cs-t4", "cs-t9"),
            ];

            for (position_slot, blend_slot) in slot_pairs {
                if self.is_position_blend_slot_match(
                    &pointlist_index,
                    position_stride,
                    blend_stride,
                    vertex_count,
                    position_slot,
                    blend_slot,
                )? {
                    let mut wrapper = D3D11GameTypeWrapper::new(d3d11_game_type.clone());
                    wrapper.position_extract_slot = position_slot.to_string();
                    wrapper.blend_extract_slot = blend_slot.to_string();
                    wrapper.position_extract_index = pointlist_index.clone();
                    wrapper.blend_extract_index = pointlist_index.clone();

                    crate::extract_log!(
                        "extract_slot: position_slot={}, blend_slot={}",
                        position_slot,
                        blend_slot
                    );

                    possible_list.push(wrapper);
                    break;
                }
            }
        }

        Ok(possible_list)
    }

    pub fn auto_gametype_detect_d50694eedd2a8595(
        &self,
        pointlist_index: String,
        trianglelist_index_list: Vec<String>,
    ) -> Result<Vec<D3D11GameTypeWrapper>, String> {
        let mut possible_list: Vec<D3D11GameTypeWrapper> = Vec::new();

        if pointlist_index.is_empty() {
            return Ok(possible_list);
        }

        for d3d11_game_type in self
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if !d3d11_game_type.gpu_pre_skinning {
                continue;
            }

            if !d3d11_game_type
                .category_slot_dict
                .values()
                .any(|v| v == "vb1")
            {
                continue;
            }

            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa_data,
                &trianglelist_index_list,
                d3d11_game_type,
            );
            if trianglelist_index.is_empty() {
                continue;
            }

            let vb1_search_key = format!("{}-vb1=", trianglelist_index);
            let vb1_buffer_filename = self
                .fa_data
                .filter_first_file(&vb1_search_key, ".buf")
                .unwrap_or_default();
            if vb1_buffer_filename.is_empty() {
                continue;
            }

            let vb1_buffer_filepath = self.fa_log.get_deduped_filepath(&vb1_buffer_filename);
            if vb1_buffer_filepath.is_empty() || !Path::new(&vb1_buffer_filepath).exists() {
                continue;
            }

            let texcoord_stride = d3d11_game_type
                .category_stride_dict
                .get("Texcoord")
                .copied()
                .unwrap_or(0);
            let position_stride = d3d11_game_type
                .category_stride_dict
                .get("Position")
                .copied()
                .unwrap_or(0);
            let blend_stride = d3d11_game_type
                .category_stride_dict
                .get("Blend")
                .copied()
                .unwrap_or(0);
            if texcoord_stride == 0 || position_stride == 0 || blend_stride == 0 {
                continue;
            }

            let vb1_size = SSMTFileUtils::get_file_size(&vb1_buffer_filepath)?;
            let vertex_count = vb1_size / texcoord_stride;

            let blend_slots = [
                "cs-t0", "cs-t1", "cs-t2", "cs-t3", "cs-t4", "cs-t5", "cs-t6",
            ];
            for (idx, blend_slot) in blend_slots.iter().enumerate() {
                if !self.is_blend_slot_match(
                    &pointlist_index,
                    blend_stride,
                    vertex_count,
                    blend_slot,
                )? {
                    continue;
                }

                let uav_slot = format!("u{}", idx);
                let Some((position_index, position_slot)) = self.get_pre_position_uav_index(
                    &pointlist_index,
                    position_stride,
                    vertex_count,
                    &uav_slot,
                )?
                else {
                    continue;
                };

                let mut wrapper = D3D11GameTypeWrapper::new(d3d11_game_type.clone());
                wrapper.position_extract_slot = position_slot;
                wrapper.position_extract_index = position_index;
                wrapper.blend_extract_slot = (*blend_slot).to_string();
                wrapper.blend_extract_index = pointlist_index.clone();
                possible_list.push(wrapper);
                break;
            }
        }

        Ok(possible_list)
    }

    pub fn auto_gametype_detect_fee307b98a965c16(
        &self,
        pointlist_index: String,
        trianglelist_index_list: Vec<String>,
    ) -> Result<Vec<D3D11GameTypeWrapper>, String> {
        let mut possible_d3d11gametype_wrapper_list: Vec<D3D11GameTypeWrapper> = Vec::new();

        let mut find_atleast_one_gpu_type = false;
        //先匹配出正确的数据类型，顺便得到从哪个Slot中提取的�?
        for d3d11_game_type in self
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if !crate::extract_new::can_match_gametype(
                &pointlist_index,
                d3d11_game_type.gpu_pre_skinning,
            ) {
                crate::extract_log!("未找到PointlistIndex，跳过GPU-PreSkinning数据类型");
                continue;
            }

            let mut d3d11_gametype_wrapper = D3D11GameTypeWrapper::new(d3d11_game_type.clone());

            if find_atleast_one_gpu_type && !d3d11_game_type.gpu_pre_skinning {
                crate::extract_log!(
                    "Already found GPU-PreSkinning type, skipping CPU-PreSkinning GameType: {}",
                    d3d11_game_type.game_type_name
                );
                continue;
            }

            //获取第一个TrianglelistIndex
            let first_trianglelist_index =
                self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                    &self.fa_data,
                    &trianglelist_index_list,
                    d3d11_game_type,
                );

            if first_trianglelist_index.is_empty() {
                crate::extract_log!(
                    "GameType {} skipped: no matching TriangleListIndex found",
                    d3d11_game_type.game_type_name
                );
                continue;
            }

            //获取每个category的buffer文件

            // 可变�?String -> String
            let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
            let mut category_buf_filesize_map: HashMap<String, u64> = HashMap::new();
            let mut all_file_exists = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let topology = d3d11_game_type
                    .category_topology_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();

                // 如果该分类需要 POINTLIST 但 pointlist_index 未知，跳过整个 GameType
                if topology == "pointlist" && pointlist_index.is_empty() {
                    crate::extract_log!(
                        "GameType {} skipped: topology=pointlist but pointlist_index is empty",
                        d3d11_game_type.game_type_name
                    );
                    all_file_exists = false;
                    break;
                }

                let mut extract_index = &first_trianglelist_index;
                if topology == "pointlist" {
                    extract_index = &pointlist_index;
                }
                crate::extract_log!("ExtractIndex: {}", extract_index);

                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                if category_name == "Position" {
                    d3d11_gametype_wrapper.position_extract_index = extract_index.clone();
                    d3d11_gametype_wrapper.position_extract_slot = category_slot.clone();
                } else if category_name == "Blend" {
                    d3d11_gametype_wrapper.blend_extract_index = extract_index.clone();
                    d3d11_gametype_wrapper.blend_extract_slot = category_slot.clone();
                }

                crate::extract_log!(
                    "CategoryName: {}, CategorySlot: {}",
                    category_name,
                    category_slot
                );

                let search_key = format!("{}-{}", extract_index, category_slot);
                let category_buf_filename = self
                    .fa_data
                    .filter_first_file(&search_key, ".buf")
                    .unwrap_or_default();

                crate::extract_log!("category_buf_filename: {}", category_buf_filename);

                category_buf_filename_map
                    .insert(category_name.clone(), category_buf_filename.clone());

                let category_buf_filepath = self.resolve_fa_filepath(&category_buf_filename);
                if !category_buf_filepath.exists() {
                    crate::extract_log!(
                        "Buffer file not found for category {}: {}",
                        category_name,
                        category_buf_filepath.display()
                    );
                    all_file_exists = false;
                    break;
                }

                let category_buf_txt_filename = self
                    .fa_data
                    .filter_first_file(&search_key, ".txt")
                    .unwrap_or_default();
                let category_buf_txt_filepath =
                    self.resolve_fa_filepath(&category_buf_txt_filename);

                // Same total stride is not enough to identify a GPU trianglelist
                // category. Verify semantic/index/format/offset against the captured
                // D3D11 InputLayout when element[] metadata is available.
                if d3d11_game_type.gpu_pre_skinning
                    && topology != "pointlist"
                    && !category_buf_txt_filename.is_empty()
                    && category_buf_txt_filepath.exists()
                    && !self.category_matches_migoto_input_layout(
                        d3d11_game_type,
                        category_name,
                        &category_slot,
                        &category_buf_txt_filepath,
                    )?
                {
                    crate::extract_log!(
                        "GameType {} skipped: captured InputLayout does not match category {} in slot {}",
                        d3d11_game_type.game_type_name,
                        category_name,
                        category_slot
                    );
                    all_file_exists = false;
                    break;
                }

                let category_buf_filesize = if category_buf_txt_filename.is_empty()
                    || !category_buf_txt_filepath.exists()
                {
                    SSMTFileUtils::get_file_size(&category_buf_filepath)?
                } else {
                    if d3d11_game_type.gpu_pre_skinning {
                        SSMTFileUtils::get_file_size(&category_buf_filepath)?
                    } else {
                        SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_buf_txt_filepath)?
                    }
                };
                category_buf_filesize_map.insert(category_name.clone(), category_buf_filesize);
            }

            if !all_file_exists {
                crate::extract_log!(
                    "GameType {} skipped: missing buffer files for one or more categories",
                    d3d11_game_type.game_type_name
                );
                continue;
            }

            let mut vertex_number = 0;
            // GPU pre-skinning Position buffers can be views into a larger shared
            // backing resource. For those buffers, filesize / stride is the resource
            // capacity rather than this draw's logical vertex count. Defer Position
            // validation until Texcoord/Blend have established the logical count.
            let mut gpu_position_vertex_capacity: Option<u64> = None;
            let mut all_match = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let category_stride = d3d11_game_type
                    .category_stride_dict
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);
                let category_buf_filesize = category_buf_filesize_map
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);

                let tmp_vertex_number = if category_stride > 0 {
                    category_buf_filesize / category_stride
                } else {
                    0
                };

                if tmp_vertex_number == 0 {
                    crate::extract_log!(
                        "GameType {} skipped: invalid stride or empty buffer for category {}",
                        d3d11_game_type.game_type_name,
                        category_name
                    );
                    all_match = false;
                    break;
                } else {
                    crate::extract_log!("tmp_vertex_number: category_buf_filesize / category_stride = {} for category {}", tmp_vertex_number, category_name);
                }

                if d3d11_game_type.gpu_pre_skinning && category_name == "Position" {
                    gpu_position_vertex_capacity = Some(tmp_vertex_number);
                    crate::extract_log!(
                        "GPU Position resource capacity: {} vertices; defer logical vertex-count comparison until non-Position categories are matched",
                        tmp_vertex_number
                    );
                    continue;
                }

                if !d3d11_game_type.gpu_pre_skinning {
                    let yushu = category_buf_filesize % category_stride;
                    if yushu != 0 {
                        crate::extract_log!("GameType {} skipped: buffer filesize {} is not a multiple of stride {} for category {}", d3d11_game_type.game_type_name, category_buf_filesize, category_stride, category_name);
                        all_match = false;
                        break;
                    }
                }

                if !d3d11_game_type.gpu_pre_skinning {
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();

                    let search_str = format!("{}-{}", first_trianglelist_index, category_slot);
                    let category_txt_filename =
                        self.fa_data.filter_first_file(&search_str, ".txt")?;
                    if category_txt_filename.is_empty() {
                        crate::extract_log!(
                            "TXT file not found for category {}: search key {}",
                            category_name,
                            search_str
                        );
                        all_match = false;
                        break;
                    } else {
                        let category_txt_filepath =
                            self.resolve_fa_filepath(&category_txt_filename);
                        let vertex_count_in_txt = SSMTFileUtils::find_migoto_ini_attribute_in_file(
                            &category_txt_filepath,
                            "vertex count",
                        )?;
                        let vertex_count_in_txt_int =
                            vertex_count_in_txt.parse::<u64>().unwrap_or(0);
                        crate::extract_log!(
                            "vertex_count_in_txt: {}, vertex_number: {}",
                            vertex_count_in_txt_int,
                            vertex_number
                        );

                        if tmp_vertex_number < vertex_count_in_txt_int {
                            crate::extract_log!("GameType {} skipped: vertex number {} from buffer is less than vertex count {} in txt for category {}", d3d11_game_type.game_type_name, tmp_vertex_number, vertex_count_in_txt_int, category_name);
                            all_match = false;
                            break;
                        }

                        if d3d11_game_type.category_slot_dict.len() > 1 {
                            if vertex_count_in_txt_int != tmp_vertex_number {
                                crate::extract_log!("GameType {} skipped: vertex number {} from buffer does not match vertex count {} in txt for category {} in multi-slot GameType", d3d11_game_type.game_type_name, tmp_vertex_number, vertex_count_in_txt_int, category_name);
                                all_match = false;
                                break;
                            }
                        }
                    }
                }

                crate::extract_log!("category: {}", category_name);
                crate::extract_log!(
                    "vertex_number: {}, tmp_vertex_number: {} for category {}",
                    vertex_number,
                    tmp_vertex_number,
                    category_name
                );

                if vertex_number == 0 {
                    vertex_number = tmp_vertex_number;
                } else if vertex_number != tmp_vertex_number {
                    crate::extract_log!("GameType {} skipped: vertex number {} from category {} does not match vertex number {} from previous categories", d3d11_game_type.game_type_name, tmp_vertex_number, category_name, vertex_number);
                    all_match = false;
                    break;
                } else {
                    crate::extract_log!(
                        "category match successful: vertex number {} from category {}",
                        vertex_number,
                        category_name
                    );
                }
            }

            if all_match && d3d11_game_type.gpu_pre_skinning {
                if let Some(position_vertex_capacity) = gpu_position_vertex_capacity {
                    if vertex_number != 0 && position_vertex_capacity < vertex_number {
                        crate::extract_log!(
                            "GameType {} skipped: Position resource capacity {} is less than logical vertex number {} from non-Position categories",
                            d3d11_game_type.game_type_name,
                            position_vertex_capacity,
                            vertex_number
                        );
                        all_match = false;
                    } else if vertex_number != 0 {
                        crate::extract_log!(
                            "GPU Position capacity match successful: resource capacity {} >= logical vertex number {}",
                            position_vertex_capacity,
                            vertex_number
                        );
                    }
                }
            }

            if all_match {
                possible_d3d11gametype_wrapper_list.push(d3d11_gametype_wrapper);
                crate::extract_log!("Matched Gametype: {}", d3d11_game_type.game_type_name);
            }

            //如果找到了一个GPUPreSkinning就标记一下，这样后面就不会匹配CPU类型了�?
            if !find_atleast_one_gpu_type {
                for d3d11_gametype_wrapper in possible_d3d11gametype_wrapper_list.iter() {
                    if d3d11_gametype_wrapper.d3d11_game_type.gpu_pre_skinning {
                        crate::extract_log!("Found GPU-PreSkinning GameType: {}, will skip remaining CPU-PreSkinning types", d3d11_gametype_wrapper.d3d11_game_type.game_type_name);
                        find_atleast_one_gpu_type = true;
                        break;
                    }
                }
            }
        }

        Ok(possible_d3d11gametype_wrapper_list)
    }

    pub fn extract_model_new(
        &self,
        draw_ib: String,
        d3d11_gametype_wrapper_list: Vec<D3D11GameTypeWrapper>,
        pointlist_index: String,
        trianglelist_index_list: Vec<String>,
    ) -> Result<(), String> {
        if d3d11_gametype_wrapper_list.is_empty() {
            crate::extract_new::log_skipped_drawib(
                &draw_ib,
                format!(
                    "no matching GameType found. PointlistIndex: {} TriangleListIndex: {:?}",
                    pointlist_index, trianglelist_index_list
                ),
            );
            return Ok(());
        }

        let mut precollected_ib_candidates: PrecollectedIbCandidateMap = BTreeMap::new();

        for trianglelist_index in trianglelist_index_list.iter() {
            let search_key = format!("{}-ib", trianglelist_index);
            let ib_txt_filename = self.fa_data.filter_first_file(&search_key, ".txt")?;
            if ib_txt_filename.is_empty() {
                continue;
            }

            crate::extract_log!(
                "Found IB txt file: {}, for TriangleListIndex: {}",
                ib_txt_filename,
                trianglelist_index
            );

            let ib_txt_filepath = self.fa_log.get_deduped_filepath(&ib_txt_filename);
            crate::extract_log!("IB txt filepath: {}", ib_txt_filepath);

            let ib_txt_file = IndexBufferTxtFile::new(ib_txt_filepath, false)?;
            let first_index_u64: u64 = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            let index_count_u64: u64 = ib_txt_file.index_count.parse::<u64>().unwrap_or(0);

            crate::extract_log!(
                "FirstIndex: {}, IndexCount: {}",
                ib_txt_file.first_index,
                ib_txt_file.index_count
            );

            Self::add_precollected_ib_candidate(
                &mut precollected_ib_candidates,
                first_index_u64,
                index_count_u64,
                ib_txt_filename.clone(),
            );
        }

        crate::extract_log!("------------------------------------------------");

        //输出查看一下
        for ((first_index, index_count), candidates) in &precollected_ib_candidates {
            crate::extract_log!(
                "DrawCall candidates: FirstIndex={}, IndexCount={}, IB txt filenames={:?}",
                first_index,
                index_count,
                candidates
            );
        }

        for d3d11_gametype_wrapper in d3d11_gametype_wrapper_list.iter() {
            let d3d11_gametype_wrapper: &D3D11GameTypeWrapper = d3d11_gametype_wrapper; // 显式类型
            crate::extract_log!(
                "Extracting with GameType: {}",
                d3d11_gametype_wrapper.d3d11_game_type.game_type_name
            );

            crate::extract_log!(
                "PositionSlot: {}",
                d3d11_gametype_wrapper.position_extract_slot.clone()
            );
            crate::extract_log!(
                "PositionIndex: {}",
                d3d11_gametype_wrapper.position_extract_index.clone()
            );
            crate::extract_log!(
                "BlendSlot: {}",
                d3d11_gametype_wrapper.blend_extract_slot.clone()
            );
            crate::extract_log!(
                "BlendIndex: {}",
                d3d11_gametype_wrapper.blend_extract_index.clone()
            );

            let mut d3d11_gametype: D3D11GameType = d3d11_gametype_wrapper.d3d11_game_type.clone();

            //设置一下CategorySlot，因为数据类型文件里可能是空着�?
            d3d11_gametype.category_slot_dict.insert(
                "Position".to_string(),
                d3d11_gametype_wrapper.position_extract_slot.clone(),
            );
            d3d11_gametype.category_slot_dict.insert(
                "Blend".to_string(),
                d3d11_gametype_wrapper.blend_extract_slot.clone(),
            );

            //获取TrianglelistIndex
            let trianglelist_index = self.d3d11_gametype_lv2.filter_trianglelist_index_unity_vs(
                &self.fa_data,
                &trianglelist_index_list,
                &d3d11_gametype.clone(),
            );
            crate::extract_log!("Using TriangleListIndex: {}", trianglelist_index);

            let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
            for category_name in d3d11_gametype.ordered_category_name_list.iter() {
                let topology = d3d11_gametype
                    .category_topology_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();

                let mut extract_index = trianglelist_index.clone();
                if topology == "pointlist" && pointlist_index != "" {
                    extract_index = pointlist_index.clone();

                    if category_name == "Position" {
                        extract_index = d3d11_gametype_wrapper.position_extract_index.clone();
                    } else if category_name == "Blend" {
                        extract_index = d3d11_gametype_wrapper.blend_extract_index.clone();
                    }
                }
                crate::extract_log!("Final ExtractIndex: {}", extract_index);

                let mut category_slot = d3d11_gametype
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                if category_name == "Position" {
                    category_slot = d3d11_gametype_wrapper.position_extract_slot.clone();
                } else if category_name == "Blend" {
                    category_slot = d3d11_gametype_wrapper.blend_extract_slot.clone();
                }
                crate::extract_log!("Final CategorySlot: {}", category_slot);

                //获取对应的文件名
                let search_key = format!("{}-{}=", extract_index, category_slot);
                let category_buf_filename = self.fa_data.filter_first_file(&search_key, ".buf")?;
                crate::extract_log!("CategoryBufFileName: {}", category_buf_filename);

                category_buf_filename_map
                    .insert(category_name.clone(), category_buf_filename.clone());
            }

            let vb0_filename_search_key = format!("{}-vb0", trianglelist_index);
            let vb0_filename = self
                .fa_data
                .filter_first_file(&vb0_filename_search_key, ".txt")?;
            let vertex_limit_vb = if vb0_filename.is_empty() {
                String::new()
            } else {
                SSMTStringUtils::substring(&vb0_filename, 11, 8)?
            };

            crate::extract_log!("VertexLimitVB: {}", vertex_limit_vb);
            let cs_output_vertex_limit_vb = Self::get_cs_output_vertex_limit_vb_from_lines(
                &self.fa_log.lines,
                &trianglelist_index,
                &vertex_limit_vb,
            )
            .unwrap_or_default();
            crate::extract_log!("CSOutputVertexLimitVB: {}", cs_output_vertex_limit_vb);

            self.export_precollected_submeshes(
                "SRMI",
                &draw_ib,
                &vertex_limit_vb,
                &cs_output_vertex_limit_vb,
                &pointlist_index,
                &d3d11_gametype,
                &d3d11_gametype_wrapper,
                &precollected_ib_candidates,
            )?;
        }

        Ok(())
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        // keep for future use if needed
        let _workspace_base = PathBuf::from(&self.workspace_path);

        let draw_ib_list = if self.specify_drawib_extract {
            self.drawib_config
                .entries
                .iter()
                .map(|entry| entry.draw_ib.trim().to_string())
                .filter(|draw_ib| !draw_ib.is_empty())
                .collect::<Vec<String>>()
        } else {
            let mut draw_ib_set: BTreeSet<String> = BTreeSet::new();
            for ib_txt_file_name in self.fa_data.filter_filelist("-ib=", ".txt") {
                let draw_ib = ib_txt_file_name.get(10..18).unwrap_or_default().to_string();
                if draw_ib.len() == 8 {
                    draw_ib_set.insert(draw_ib);
                }
            }
            draw_ib_set.into_iter().collect::<Vec<String>>()
        };

        //对每个DrawIB依次提取模型
        for draw_ib in draw_ib_list.iter() {
            //打印当前处理的DrawIB和Alias
            crate::extract_log!("DrawIB: {}", draw_ib);

            //根据FrameAnalysis下面的log.txt，找到PointlistIndex
            let pointlist_index: String = self.get_copyresource_aware_pointlist_index(&draw_ib);
            crate::extract_log!("PointlistIndex: {:?}", pointlist_index);

            if pointlist_index.is_empty() {
                print!("未找到对应的PointlistIndex，该DrawIB对应数据类型可能为CPU-PreSkinning类型")
            }

            //获取当前DrawIB对应的TriangleListIndex列表
            let trianglelist_index_list: Vec<String> =
                self.fa_data.get_trianglelist_index_list(&draw_ib);

            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TriangleListIndex: {}", trianglelist_index);
            }
            /*
                * 接下来要判断是否是脸部和头发的特殊Shader
                因为在崩�?.2版本更新后，有多种ComputeShader分别负责不同部分的渲染�?
                - 脸部、头�?1c932707d4d8df41
                - 身体
                - 组队界面多角色同时渲�?1c932707d4d8df41
                - NPC集体渲染
            */
            //拿到cs-cb0的文件名，判断是否包含指定的Hash�?
            let cs_cb0_key: String = format!("{}-cs-cb0=", pointlist_index);
            crate::extract_log!("cs-cb0 key: {}", cs_cb0_key);
            let cs_cb0_filename: String = self.fa_data.filter_first_file(&cs_cb0_key, ".buf")?;
            crate::extract_log!("cs-cb0 filename: {}", cs_cb0_filename);
            crate::extract_log!();

            if pointlist_index.is_empty() {
                crate::extract_log!("PointlistIndex is empty, skipping special shader detection and using general extraction");
                crate::extract_log!("执行提取:通用提取fee307b98a965c16");
                let mut possible_d3d11gametype_wrapper_list = self
                    .auto_gametype_detect_fee307b98a965c16(
                        pointlist_index.clone(),
                        trianglelist_index_list.clone(),
                    )?;
                possible_d3d11gametype_wrapper_list.retain(|wrapper| {
                    data_type_filter.allows(wrapper.d3d11_game_type.gpu_pre_skinning)
                });
                crate::extract_log!(
                    "possible_d3d11gametype_wrapper_list count: {}",
                    possible_d3d11gametype_wrapper_list.len()
                );
                for d3d11gametype_wrapper in possible_d3d11gametype_wrapper_list.iter() {
                    crate::extract_log!(
                        "Matched GameType: {}",
                        d3d11gametype_wrapper.d3d11_game_type.game_type_name
                    );
                }

                self.extract_model_new(
                    draw_ib.to_string(),
                    possible_d3d11gametype_wrapper_list,
                    pointlist_index.clone(),
                    trianglelist_index_list.clone(),
                )?;
            } else {
                if cs_cb0_filename.contains("1c932707d4d8df41")
                    || cs_cb0_filename.contains("4d9c23fd387846c7")
                    || cs_cb0_filename.contains("ba3d8ab37ea2fd2d")
                {
                    crate::extract_log!(
                        "提取执行: 1c932707d4d8df41_4d9c23fd387846c7_ba3d8ab37ea2fd2d"
                    );
                    let mut possible_d3d11gametype_wrapper_list = self
                        .auto_gametype_detect_1c932707d4d8df41_4d9c23fd387846c7(
                            pointlist_index.clone(),
                            trianglelist_index_list.clone(),
                        )?;
                    possible_d3d11gametype_wrapper_list.retain(|wrapper| {
                        data_type_filter.allows(wrapper.d3d11_game_type.gpu_pre_skinning)
                    });
                    crate::extract_log!(
                        "possible_d3d11gametype_wrapper_list count: {}",
                        possible_d3d11gametype_wrapper_list.len()
                    );
                    self.extract_model_new(
                        draw_ib.to_string(),
                        possible_d3d11gametype_wrapper_list,
                        pointlist_index.clone(),
                        trianglelist_index_list.clone(),
                    )?;
                } else if cs_cb0_filename.contains("d50694eedd2a8595") {
                    crate::extract_log!("提取执行: d50694eedd2a8595");
                    let mut possible_d3d11gametype_wrapper_list = self
                        .auto_gametype_detect_d50694eedd2a8595(
                            pointlist_index.clone(),
                            trianglelist_index_list.clone(),
                        )?;
                    possible_d3d11gametype_wrapper_list.retain(|wrapper| {
                        data_type_filter.allows(wrapper.d3d11_game_type.gpu_pre_skinning)
                    });
                    crate::extract_log!(
                        "possible_d3d11gametype_wrapper_list count: {}",
                        possible_d3d11gametype_wrapper_list.len()
                    );
                    self.extract_model_new(
                        draw_ib.to_string(),
                        possible_d3d11gametype_wrapper_list,
                        pointlist_index.clone(),
                        trianglelist_index_list.clone(),
                    )?;
                } else if cs_cb0_filename.contains("c9f2b46571d22858") {
                    crate::extract_log!("提取执行: c9f2b46571d22858");
                    let mut possible_d3d11gametype_wrapper_list = self
                        .auto_gametype_detect_1c932707d4d8df41_4d9c23fd387846c7(
                            pointlist_index.clone(),
                            trianglelist_index_list.clone(),
                        )?;
                    possible_d3d11gametype_wrapper_list.retain(|wrapper| {
                        data_type_filter.allows(wrapper.d3d11_game_type.gpu_pre_skinning)
                    });
                    crate::extract_log!(
                        "possible_d3d11gametype_wrapper_list count: {}",
                        possible_d3d11gametype_wrapper_list.len()
                    );
                    self.extract_model_new(
                        draw_ib.to_string(),
                        possible_d3d11gametype_wrapper_list,
                        pointlist_index.clone(),
                        trianglelist_index_list.clone(),
                    )?;
                } else {
                    crate::extract_log!("执行提取:通用提取fee307b98a965c16");
                    let mut possible_d3d11gametype_wrapper_list = self
                        .auto_gametype_detect_fee307b98a965c16(
                            pointlist_index.clone(),
                            trianglelist_index_list.clone(),
                        )?;
                    possible_d3d11gametype_wrapper_list.retain(|wrapper| {
                        data_type_filter.allows(wrapper.d3d11_game_type.gpu_pre_skinning)
                    });
                    crate::extract_log!(
                        "possible_d3d11gametype_wrapper_list count: {}",
                        possible_d3d11gametype_wrapper_list.len()
                    );
                    for d3d11gametype_wrapper in possible_d3d11gametype_wrapper_list.iter() {
                        crate::extract_log!(
                            "Matched GameType: {}",
                            d3d11gametype_wrapper.d3d11_game_type.game_type_name
                        );
                    }

                    self.extract_model_new(
                        draw_ib.to_string(),
                        possible_d3d11gametype_wrapper_list,
                        pointlist_index.clone(),
                        trianglelist_index_list.clone(),
                    )?;
                }
            }
        }

        self.sync_workspace_deduped_textures_and_json(&draw_ib_list)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{PrecollectedIbCandidateMap, SRMINewExtractor};
    use crate::common::d3d11_element::D3D11Element;
    use crate::common::d3d11_gametype::D3D11GameType;
    use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
    use crate::common::d3d11_gametype_wrapper::D3D11GameTypeWrapper;
    use crate::common::frame_analysis::frameanalysis_data::FrameAnalysisData;
    use crate::common::frame_analysis::frameanalysis_log::FrameAnalysisSingleLog;
    use crate::config::drawib_config::DrawIBConfig;
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_candidate_test_extractor(files: &[&str]) -> (SRMINewExtractor, std::path::PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ssmt-srmi-drawcall-candidates-{}-{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).expect("failed to create candidate test directory");
        let deduped_dir = dir.join("deduped");
        fs::create_dir_all(&deduped_dir).expect("failed to create deduped test directory");

        let mut deduped_file_map = HashMap::new();
        for file_name in files {
            fs::write(dir.join(file_name), b"test").expect("failed to create candidate test file");
            fs::write(deduped_dir.join(file_name), b"test")
                .expect("failed to create deduped candidate test file");
            deduped_file_map.insert(file_name.to_string(), file_name.to_string());
        }

        let fa_data = FrameAnalysisData {
            dir: dir.clone(),
            files: files
                .iter()
                .map(|file_name| file_name.to_string())
                .collect(),
        };
        let extractor = SRMINewExtractor {
            fa_data,
            fa_log: FrameAnalysisSingleLog {
                dir: dir.to_string_lossy().to_string(),
                log_filename: String::new(),
                lines: Vec::new(),
                filename_deduped_filename_map: deduped_file_map,
            },
            workspace_path: String::new(),
            drawib_config: DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            },
            specify_drawib_extract: false,
            d3d11_gametype_lv2: D3D11GameTypeLv2::default(),
        };

        (extractor, dir)
    }

    fn two_slot_trianglelist_game_type() -> D3D11GameType {
        let mut game_type = D3D11GameType::default();
        game_type.game_type_name = "TestTwoSlot".to_string();
        game_type.ordered_category_name_list = vec!["Normal".to_string(), "Texcoord".to_string()];
        game_type
            .category_slot_dict
            .insert("Normal".to_string(), "vb0".to_string());
        game_type
            .category_slot_dict
            .insert("Texcoord".to_string(), "vb1".to_string());
        game_type
            .category_topology_dict
            .insert("Normal".to_string(), "trianglelist".to_string());
        game_type
            .category_topology_dict
            .insert("Texcoord".to_string(), "trianglelist".to_string());
        game_type
    }

    #[test]
    fn precollected_candidates_preserve_same_first_index_and_index_count() {
        let mut candidates = PrecollectedIbCandidateMap::new();
        SRMINewExtractor::add_precollected_ib_candidate(
            &mut candidates,
            65430,
            441,
            "000119-ib=3b4647d4.txt".to_string(),
        );
        SRMINewExtractor::add_precollected_ib_candidate(
            &mut candidates,
            65430,
            441,
            "000148-ib=3b4647d4.txt".to_string(),
        );

        assert_eq!(candidates.get(&(65430, 441)).map(Vec::len), Some(2));
    }

    #[test]
    fn candidate_selection_skips_drawcall_missing_required_vb() {
        let files = [
            "000148-ib=3b4647d4.txt",
            "000148-ib=3b4647d4.buf",
            "000148-vb0=aaaaaaaa.buf",
            "000119-ib=3b4647d4.txt",
            "000119-ib=3b4647d4.buf",
            "000119-vb0=bbbbbbbb.buf",
            "000119-vb1=dce61e19.buf",
        ];
        let (extractor, dir) = create_candidate_test_extractor(&files);
        let game_type = two_slot_trianglelist_game_type();
        let mut wrapper = D3D11GameTypeWrapper::new(game_type.clone());
        wrapper.position_extract_slot = "vb0".to_string();

        let invalid_error = extractor
            .select_precollected_candidate(
                65430,
                441,
                &["000148-ib=3b4647d4.txt".to_string()],
                "",
                &game_type,
                &wrapper,
            )
            .expect_err("candidate without vb1 must be rejected");
        assert!(invalid_error.contains("FirstIndex=65430, IndexCount=441"));
        assert!(invalid_error.contains("DrawCall 000148"));
        assert!(invalid_error.contains("Category=Texcoord VB=vb1"));

        let (selected, category_files) = extractor
            .select_precollected_candidate(
                65430,
                441,
                &[
                    "000148-ib=3b4647d4.txt".to_string(),
                    "000119-ib=3b4647d4.txt".to_string(),
                ],
                "",
                &game_type,
                &wrapper,
            )
            .expect("second candidate contains every required VB");

        assert!(selected.starts_with("000119-"));
        assert_eq!(
            category_files.get("Texcoord").map(String::as_str),
            Some("000119-vb1=dce61e19.buf")
        );

        fs::remove_dir_all(dir).expect("failed to remove candidate test directory");
    }

    #[test]
    fn candidate_selection_preserves_stride_compatible_gametype_interpretations() {
        let files = [
            "000009-ib=c5794477.txt",
            "000009-ib=c5794477.buf",
            "000009-vb0=e6d1f87b.buf",
            "000009-vb1=5e51291b.buf",
            "000009-vb1=5e51291b.txt",
        ];
        let (extractor, dir) = create_candidate_test_extractor(&files);
        let incompatible_active_layout = "element[0]:\n  SemanticName: COLOR\n  SemanticIndex: 0\n  Format: R8G8B8A8_UNORM\n  InputSlot: 1\n  AlignedByteOffset: 0\n";
        fs::write(
            dir.join("000009-vb1=5e51291b.txt"),
            incompatible_active_layout,
        )
        .expect("failed to write active InputLayout fixture");
        fs::write(
            dir.join("deduped").join("000009-vb1=5e51291b.txt"),
            incompatible_active_layout,
        )
        .expect("failed to write deduped InputLayout fixture");

        let mut game_type = D3D11GameType::from_parts(
            "GPU_AlternateT8Interpretation",
            vec![
                D3D11Element::new(
                    "POSITION",
                    "R32G32B32_FLOAT",
                    "vb0",
                    "trianglelist",
                    "Position",
                    "Position",
                    "12",
                ),
                D3D11Element::new(
                    "TEXCOORD",
                    "R32G32_FLOAT",
                    "vb1",
                    "trianglelist",
                    "Texcoord",
                    "Texcoord",
                    "8",
                ),
            ],
        );
        game_type.gpu_pre_skinning = true;
        let mut wrapper = D3D11GameTypeWrapper::new(game_type.clone());
        wrapper.position_extract_slot = "vb0".to_string();

        let (selected, category_files) = extractor
            .select_precollected_candidate(
                0,
                23205,
                &["000009-ib=c5794477.txt".to_string()],
                "",
                &game_type,
                &wrapper,
            )
            .expect("alternate GameType interpretation must remain exportable");

        assert_eq!(selected, "000009-ib=c5794477.txt");
        assert_eq!(
            category_files.get("Texcoord").map(String::as_str),
            Some("000009-vb1=5e51291b.buf")
        );

        fs::remove_dir_all(dir).expect("failed to remove alternate GameType test directory");
    }

    #[test]
    fn copyresource_pointlist_lookup_tracks_source_hash_chain() {
        let lines = vec![
            "000006 CSSetShaderResources(StartSlot:0, NumViews:1)".to_string(),
            "       0: view=0x1 resource=0x2 hash=4ecab248".to_string(),
            "000007 CopyResource(pDstResource:0x1, pSrcResource:0x2)".to_string(),
            "       Src: resource=0x2 hash=4ecab248".to_string(),
            "       Dst: resource=0x1 hash=43cad7d1".to_string(),
            "000009 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)"
                .to_string(),
            "       0: resource=0x1 hash=43cad7d1".to_string(),
            "000009 IASetIndexBuffer(pIndexBuffer:0x1, Format:57, Offset:0) hash=6c751ecd"
                .to_string(),
        ];
        let drawcall_index_list = vec!["000009".to_string()];

        let pointlist_index = SRMINewExtractor::get_copyresource_aware_pointlist_index_from_lines(
            &lines,
            &drawcall_index_list,
        );

        assert_eq!(pointlist_index.as_deref(), Some("000006"));
    }

    #[test]
    fn copyresource_pointlist_lookup_preserves_direct_match_behavior() {
        let lines = vec![
            "000005 CSSetShaderResources(StartSlot:0, NumViews:1)".to_string(),
            "       0: view=0x1 resource=0x2 hash=43cad7d1".to_string(),
            "000009 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)"
                .to_string(),
            "       0: resource=0x1 hash=43cad7d1".to_string(),
            "000009 IASetIndexBuffer(pIndexBuffer:0x1, Format:57, Offset:0) hash=6c751ecd"
                .to_string(),
        ];
        let drawcall_index_list = vec!["000009".to_string()];

        let pointlist_index = SRMINewExtractor::get_copyresource_aware_pointlist_index_from_lines(
            &lines,
            &drawcall_index_list,
        );

        assert_eq!(pointlist_index.as_deref(), Some("000005"));
    }

    #[test]
    fn copyresource_pointlist_lookup_groups_same_index_lines_into_one_block() {
        let lines = vec![
            "000006 CSSetShaderResources(StartSlot:0, NumViews:1)".to_string(),
            "       0: view=0x1 resource=0x2 hash=4ecab248".to_string(),
            "000007 CopyResource(pDstResource:0x1, pSrcResource:0x2)".to_string(),
            "       Src: resource=0x2 hash=4ecab248".to_string(),
            "       Dst: resource=0x1 hash=43cad7d1".to_string(),
            "000009 PSSetShaderResources(StartSlot:4, NumViews:1, ppShaderResourceViews:0x1)"
                .to_string(),
            "       4: view=0x1 resource=0x2 hash=bb506308".to_string(),
            "000009 IASetVertexBuffers(StartSlot:0, NumBuffers:8, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)"
                .to_string(),
            "       0: resource=0x1 hash=43cad7d1".to_string(),
            "000009 IASetInputLayout(pInputLayout:0x1)".to_string(),
            "000009 IASetIndexBuffer(pIndexBuffer:0x1, Format:57, Offset:0) hash=6c751ecd"
                .to_string(),
        ];
        let drawcall_index_list = vec!["000009".to_string()];

        let pointlist_index = SRMINewExtractor::get_copyresource_aware_pointlist_index_from_lines(
            &lines,
            &drawcall_index_list,
        );

        assert_eq!(pointlist_index.as_deref(), Some("000006"));
    }

    #[test]
    fn cs_output_vertex_limit_vb_tracks_copyresource_source_hash() {
        let lines = vec![
            "000006 Dispatch(ThreadGroupCountX:375, ThreadGroupCountY:1, ThreadGroupCountZ:1)"
                .to_string(),
            "000007 CopyResource(pDstResource:0x1, pSrcResource:0x2)".to_string(),
            "       Src: resource=0x2 hash=14114504".to_string(),
            "       Dst: resource=0x1 hash=1911209d".to_string(),
            "000021 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)"
                .to_string(),
            "       0: resource=0x1 hash=1911209d".to_string(),
            "000021 IASetIndexBuffer(pIndexBuffer:0x1, Format:57, Offset:0) hash=d2b9aa51"
                .to_string(),
        ];

        let cs_output_vertex_limit_vb = SRMINewExtractor::get_cs_output_vertex_limit_vb_from_lines(
            &lines, "000021", "1911209d",
        );

        assert_eq!(cs_output_vertex_limit_vb.as_deref(), Some("14114504"));
    }

    #[test]
    fn cs_output_vertex_limit_vb_ignores_direct_vb_without_copyresource() {
        let lines = vec![
            "000021 IASetVertexBuffers(StartSlot:0, NumBuffers:1, ppVertexBuffers:0x1, pStrides:0x1, pOffsets:0x1)"
                .to_string(),
            "       0: resource=0x1 hash=1911209d".to_string(),
            "000021 IASetIndexBuffer(pIndexBuffer:0x1, Format:57, Offset:0) hash=d2b9aa51"
                .to_string(),
        ];

        let cs_output_vertex_limit_vb = SRMINewExtractor::get_cs_output_vertex_limit_vb_from_lines(
            &lines, "000021", "1911209d",
        );

        assert_eq!(cs_output_vertex_limit_vb, None);
    }
}
