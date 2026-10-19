use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::common::bone_matrix_buf_file::BoneMatrixBufFile;
use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::common::shape_keys::ShapeKeys;
use crate::config::drawib_config::{DrawIBConfig, DrawIBEntry};
use crate::config::path_manager::PathManager;
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::utils::ssmt_string_utils::SSMTStringUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshIndexBuffer, SubMeshJson, SubMeshTextureMarkUpInfo,
};

#[derive(Debug, Clone)]
pub struct D3D11GameTypeWrapper {
    pub d3d11gametype: D3D11GameType,
    pub pointlist_index: String,
    pub matched_trianglelistindex: String,
}

impl D3D11GameTypeWrapper {
    pub fn new(
        d3d11gametype: D3D11GameType,
        pointlist_index: impl Into<String>,
        matched_trianglelistindex: impl Into<String>,
    ) -> Self {
        Self {
            d3d11gametype,
            pointlist_index: pointlist_index.into(),
            matched_trianglelistindex: matched_trianglelistindex.into(),
        }
    }
}

fn push_unique_string(target: &mut Vec<String>, value: &str) {
    if value.is_empty() || target.iter().any(|item| item == value) {
        return;
    }
    target.push(value.to_string());
}

pub struct NTEMINewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    draw_ib_list: Vec<String>,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl NTEMINewExtractor {
    fn get_first_matching_trianglelist_index(
        &self,
        d3d11_game_type: &D3D11GameType,
        trianglelist_index_list: &[String],
    ) -> String {
        let required_trianglelist_slots: Vec<String> = d3d11_game_type
            .category_slot_dict
            .iter()
            .filter_map(|(category, category_slot)| {
                let topology = d3d11_game_type
                    .category_topology_dict
                    .get(category)
                    .cloned()
                    .unwrap_or_default();
                if topology == "pointlist" {
                    None
                } else {
                    Some(category_slot.clone())
                }
            })
            .collect();

        if required_trianglelist_slots.is_empty() {
            return trianglelist_index_list.first().cloned().unwrap_or_default();
        }

        for trianglelist_index in trianglelist_index_list {
            let mut all_slot_found = true;
            for category_slot in &required_trianglelist_slots {
                let search_str = format!("{}-{}", trianglelist_index, category_slot);
                let category_buf_filename = self
                    .fa
                    .data
                    .filter_first_file(&search_str, ".buf")
                    .unwrap_or_default();
                if category_buf_filename.is_empty() {
                    all_slot_found = false;
                    break;
                }
            }

            if all_slot_found {
                return trianglelist_index.clone();
            }
        }

        String::new()
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
    ) -> Vec<crate::workspace::submesh_json::SubMeshD3D11Element> {
        let mut result = Vec::new();
        for element_name in &d3d11_game_type.ordered_full_element_list {
            let Some(element) = d3d11_game_type
                .element_name_d3d11_element_dict
                .get(element_name)
            else {
                continue;
            };
            if element.category == category_name {
                result.push(
                    crate::workspace::submesh_json::SubMeshD3D11Element::from_d3d11_element(
                        element,
                    ),
                );
            }
        }
        result
    }

    /// Read Blend category buffer, extract unique bone (VG) indices used by the given vertex indices.
    fn collect_submesh_vg_indices(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_buf_file_name_dict: &HashMap<String, String>,
        vertex_indices: &[u32],
    ) -> Result<Vec<i32>, String> {
        let blend_stride = match d3d11_game_type.category_stride_dict.get("Blend") {
            Some(&s) if s > 0 => s as usize,
            _ => return Ok(Vec::new()),
        };

        let blend_buf_file_name = category_buf_file_name_dict
            .get("Blend")
            .cloned()
            .unwrap_or_default();
        if blend_buf_file_name.is_empty() {
            return Ok(Vec::new());
        }

        let blend_buf_path = self.fa.log.get_deduped_filepath(&blend_buf_file_name);
        if blend_buf_path.is_empty() {
            return Ok(Vec::new());
        }

        let blend_bytes = fs::read(&blend_buf_path)
            .map_err(|e| format!("Failed to read Blend buffer {}: {}", blend_buf_path, e))?;

        // Blend buffer: first 4 bytes per vertex = BLENDINDICES (R8G8B8A8_UINT)
        let mut unique_indices: BTreeSet<i32> = BTreeSet::new();
        for &vertex_idx in vertex_indices {
            let byte_start = (vertex_idx as usize).saturating_mul(blend_stride);
            if byte_start + 4 <= blend_bytes.len() {
                for &b in &blend_bytes[byte_start..byte_start + 4] {
                    unique_indices.insert(b as i32);
                }
            }
        }

        Ok(unique_indices.into_iter().collect())
    }

    /// Compute the merge key for a bone matrix (48 bytes = 4 x float3 = 12 floats).
    fn bone_matrix_merge_key(bone_matrix_bytes: &[u8]) -> String {
        if bone_matrix_bytes.len() < 48 {
            return String::new();
        }
        let floats: Vec<f32> = bone_matrix_bytes[..48]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if floats.len() != 12 {
            return String::new();
        }
        // Round to 4 decimal places for merge comparison
        format!(
            "{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}",
            floats[0],
            floats[1],
            floats[2],
            floats[3],
            floats[4],
            floats[5],
            floats[6],
            floats[7],
            floats[8],
            floats[9],
            floats[10],
            floats[11],
        )
    }

    /// Extract 8-char hash from a migoto buf filename given the slot name.
    /// Filename format: `{6-digit-index}-{slot}={hash}-{shader-hash}.buf`
    fn extract_slot_hash_from_buf_filename(buf_file_name: &str, slot: &str) -> String {
        let prefix_len = 6 + 1 + slot.len() + 1; // "index-slot="
        buf_file_name.chars().skip(prefix_len).take(8).collect()
    }

    fn export_category_buffer(
        &self,
        category_name: &str,
        category_buf_filename: &str,
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

        let category_txt_filename =
            SSMTFileUtils::get_filename_with_new_extension(category_buf_filename, "txt")?;
        let category_txt_file_path = self.fa.log.get_deduped_filepath(&category_txt_filename);
        if !category_txt_file_path.is_empty() && Path::new(&category_txt_file_path).is_file() {
            let slice_result = (|| -> Result<bool, String> {
                let metadata =
                    SSMTBinaryUtils::read_migoto_buffer_metadata(&category_txt_file_path)?;
                if metadata.stride == 0 || metadata.vertex_count == 0 {
                    return Ok(false);
                }

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
                Ok(true)
            })();

            match slice_result {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => {
                    crate::extract_log!(
                        "NTEMI category metadata fallback: category={} buf={} txt={} reason={}",
                        category_name,
                        category_buf_filename,
                        category_txt_file_path,
                        e
                    );
                }
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

    /// For DrawIndexedInstancedIndirect calls, parse the real IndexCount and
    /// StartIndexLocation from the indirect args buffer (hash + offset found in log.txt).
    /// Collect a map: `first_index → ib_txt_file_name` for unique submeshes.
    /// Also returns a separate map of `ib_txt_file_name → (real_start_index, real_index_count)`
    /// for `DrawIndexedInstancedIndirect` calls whose headers are missing from the IB txt file.
    fn collect_match_first_index_ib_map(
        &self,
        trianglelist_index_list: &[String],
    ) -> Result<(BTreeMap<u64, String>, HashMap<String, (u64, u64)>), String> {
        let mut result: BTreeMap<u64, String> = BTreeMap::new();
        let mut indirect_params: HashMap<String, (u64, u64)> = HashMap::new();
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
            let (match_first_index, _has_indirect) = if ib_txt_file.first_index == "0"
                && ib_txt_file.index_count == "0"
            {
                crate::extract_log!(
                    "[collect] {} ib={}: first_index=0 index_count=0, trying indirect resolve",
                    trianglelist_index,
                    ib_file_name
                );
                // Headers missing — likely a DrawIndexedInstancedIndirect call.
                // Try to get the real params from the indirect args buffer.
                self.fa.log.resolve_indirect_draw_params(trianglelist_index)
                    .map(|(si, ic)| {
                        crate::extract_log!(
                            "[collect] {} -> indirect resolved si={} ic={}, inserting indirect_params",
                            trianglelist_index, si, ic
                        );
                        indirect_params.insert(ib_file_name.clone(), (si, ic));
                        (si, true)
                    })
                    .unwrap_or_else(|| {
                        crate::extract_log!(
                            "[collect] {} -> indirect resolve FAILED, falling back to key=0",
                            trianglelist_index
                        );
                        (0, false)
                    })
            } else {
                (ib_txt_file.first_index.parse::<u64>().unwrap_or(0), false)
            };
            result.insert(match_first_index, ib_file_name);
        }
        Ok((result, indirect_params))
    }

    fn export_unity_vs_submeshes(
        &self,
        draw_ib: &str,
        trianglelist_index_list: &[String],
        possible_d3d11_game_type_wrapper_list: &[D3D11GameTypeWrapper],
    ) -> Result<bool, String> {
        if possible_d3d11_game_type_wrapper_list.is_empty() {
            return Ok(false);
        }

        let (match_first_index_ib_txt_file_name_dict, indirect_draw_params) =
            self.collect_match_first_index_ib_map(trianglelist_index_list)?;

        for game_type_wrapper in possible_d3d11_game_type_wrapper_list {
            let d3d11_game_type = &game_type_wrapper.d3d11gametype;
            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);

            // --- Texture bindings (per trianglelist index, shared across submeshes) ---
            let tex_content_str = format!("{}-ps-t", game_type_wrapper.matched_trianglelistindex);
            let ps_texture_list = self.fa.data.filter_texture_filename_list(&tex_content_str);
            let mut texture_mark_list: Vec<SubMeshTextureMarkUpInfo> = Vec::new();
            for tex_filename in &ps_texture_list {
                let mark_hash = SSMTStringUtils::get_file_hash_from_file_name(tex_filename);
                let mark_slot =
                    SSMTStringUtils::get_pixel_slot_number_from_texture_file_name(tex_filename)
                        .map(|n| n.to_string())
                        .unwrap_or_default();
                let mark_name = format!("ps-t{}", mark_slot);
                let mark_type = if tex_filename.ends_with(".dds") {
                    "dds"
                } else {
                    "jpg"
                };
                texture_mark_list.push(SubMeshTextureMarkUpInfo {
                    mark_name,
                    mark_hash,
                    mark_slot,
                    mark_type: mark_type.to_string(),
                    mark_file_name: tex_filename.clone(),
                    face_sdf_channel: None,
                });
            }

            // --- ShapeKeys metadata (per pointlist index, shared across submeshes) ---
            let mut shape_keys_info = ShapeKeys::default();
            if !game_type_wrapper.pointlist_index.is_empty() {
                // cs-cb0: shape key dispatch params (first 128 u32s)
                let cb0_file_name = self
                    .fa
                    .data
                    .filter_first_file(
                        &format!("{}-cs-cb0=", game_type_wrapper.pointlist_index),
                        ".buf",
                    )
                    .unwrap_or_default();
                if !cb0_file_name.is_empty() {
                    let cb0_path = self.fa.log.get_deduped_filepath(&cb0_file_name);
                    if !cb0_path.is_empty() {
                        if let Ok(cb0_u32) = SSMTBinaryUtils::read_as_r32_uint(&cb0_path) {
                            if cb0_u32.len() >= 128 {
                                shape_keys_info.vertex_count = cb0_u32[127] as i32;
                                shape_keys_info.checksum = (cb0_u32[0]
                                    .wrapping_add(cb0_u32[1])
                                    .wrapping_add(cb0_u32[2])
                                    .wrapping_add(cb0_u32[3]))
                                    as i32;
                                shape_keys_info.dispatch_y =
                                    (shape_keys_info.vertex_count + 31) / 32;
                            }
                        }
                    }
                    shape_keys_info.offsets_hash =
                        Self::extract_slot_hash_from_buf_filename(&cb0_file_name, "cs-cb0");
                }

                // cs-t0: shape key vertex IDs
                let cs_t0_filename = self
                    .fa
                    .data
                    .filter_first_file(
                        &format!("{}-cs-t0=", game_type_wrapper.pointlist_index),
                        ".buf",
                    )
                    .unwrap_or_default();
                if !cs_t0_filename.is_empty() {
                    shape_keys_info.vertex_ids_hash =
                        Self::extract_slot_hash_from_buf_filename(&cs_t0_filename, "cs-t0");
                }

                // cs-t1: shape key vertex offsets
                let cs_t1_filename = self
                    .fa
                    .data
                    .filter_first_file(
                        &format!("{}-cs-t1=", game_type_wrapper.pointlist_index),
                        ".buf",
                    )
                    .unwrap_or_default();
                if !cs_t1_filename.is_empty() {
                    shape_keys_info.vertex_offsets_hash =
                        Self::extract_slot_hash_from_buf_filename(&cs_t1_filename, "cs-t1");
                }
            }

            // Cross-submesh merged VG tracking (per game type)
            let mut merged_bone_matrix_map: HashMap<String, i32> = HashMap::new();
            let mut merged_vg_offset = 0i32;

            for ib_txt_file_name in match_first_index_ib_txt_file_name_dict.values() {
                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                let ib_txt_file_path = self.fa.log.get_deduped_filepath(ib_txt_file_name);
                let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);
                if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;

                // For DrawIndexedInstancedIndirect calls the IB txt file is missing
                // "first index:" and "index count:" headers, so the file-level values
                // default to "0". Override them with the real draw parameters parsed
                // from the indirect args buffer.
                let (real_first_index, real_index_count) = indirect_draw_params
                    .get(ib_txt_file_name)
                    .copied()
                    .unwrap_or((0, 0));
                let effective_first_index = if real_first_index > 0 {
                    real_first_index.to_string()
                } else {
                    ib_txt_file.first_index.clone()
                };
                let effective_index_count = if real_index_count > 0 {
                    real_index_count.to_string()
                } else {
                    ib_txt_file.index_count.clone()
                };
                // Also cap the parsed index count to the real draw range
                let effective_number_count =
                    if real_index_count > 0 && real_index_count < ib_txt_file.index_number_count {
                        real_index_count
                    } else {
                        ib_txt_file.index_number_count
                    };

                let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();

                for category_name in &d3d11_game_type.ordered_category_name_list {
                    let topology = d3d11_game_type
                        .category_topology_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let extract_index = if topology == "pointlist"
                        && !game_type_wrapper.pointlist_index.is_empty()
                    {
                        game_type_wrapper.pointlist_index.clone()
                    } else {
                        game_type_wrapper.matched_trianglelistindex.clone()
                    };

                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_buf_filename = self
                        .fa
                        .data
                        .filter_first_file(&format!("{}-{}=", extract_index, category_slot), ".buf")
                        .unwrap_or_default();
                    if category_buf_filename.is_empty() {
                        return Err(format!(
                            "Failed to find category buffer: DrawIB={} GameType={} Category={} Index={} IBIndex={}",
                            draw_ib, d3d11_game_type.game_type_name, category_name, extract_index, ib_txt_file.index
                        ));
                    }
                    category_buf_filename_map.insert(category_name.clone(), category_buf_filename);
                }
                let unique_str_folder_name = format!(
                    "{}-{}-{}",
                    draw_ib, effective_index_count, effective_first_index
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
                    effective_first_index.parse::<usize>().unwrap_or(0),
                    effective_number_count as usize,
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
                        d3d11_game_type.gpu_pre_skinning,
                        &category_output_buf_file_path,
                    )?;
                }

                let vb0_hash = if let Some(position_buf_filename) =
                    category_buf_filename_map.get("POSITION")
                {
                    self.build_category_hash_from_buf_file_name(
                        d3d11_game_type,
                        "POSITION",
                        position_buf_filename,
                    )?
                } else {
                    String::new()
                };

                // --- advanced metadata ---

                // Vertex/Index counts from IB
                let vertex_offset = effective_first_index.parse::<i64>().unwrap_or(0);
                let index_offset = match_first_index_ib_txt_file_name_dict
                    .iter()
                    .find(|(_, name)| *name == ib_txt_file_name)
                    .map(|(&k, _)| k as i64)
                    .unwrap_or(0);
                let index_count = effective_number_count as i64;

                // Vertex count from Position buffer size
                let position_stride = d3d11_game_type
                    .category_stride_dict
                    .get("POSITION")
                    .copied()
                    .unwrap_or(12);
                let vertex_count =
                    if let Some(pos_buf_filename) = category_buf_filename_map.get("POSITION") {
                        let pos_path = self.fa.log.get_deduped_filepath(pos_buf_filename);
                        if pos_path.is_empty() {
                            0i64
                        } else {
                            let sz = SSMTFileUtils::get_file_size(&pos_path).unwrap_or(0);
                            (sz / position_stride) as i64
                        }
                    } else {
                        0i64
                    };

                // Extract VG data from Blend buffer
                let submesh_vg_indices: Vec<i32> = if d3d11_game_type.gpu_pre_skinning {
                    let vertex_indices: Vec<u32> = ib_buf_file.number_list.clone();
                    self.collect_submesh_vg_indices(
                        d3d11_game_type,
                        &category_buf_filename_map,
                        &vertex_indices,
                    )?
                } else {
                    Vec::new()
                };
                let submesh_vg_count: i32 = submesh_vg_indices.last().map_or(0, |&m| m + 1);

                // Extract CB4 hash and bone matrix from vs-cb4 buffer
                let cb4_file_name = self
                    .fa
                    .data
                    .filter_first_file(
                        &format!("{}-vs-cb4=", game_type_wrapper.matched_trianglelistindex),
                        ".buf",
                    )
                    .unwrap_or_default();
                let mut cb4_hash = String::new();
                let mut bone_matrix_file_name = String::new();
                if !cb4_file_name.is_empty() {
                    cb4_hash = cb4_file_name.chars().skip(14).take(8).collect();
                    let cb4_buf_path = self.fa.log.get_deduped_filepath(&cb4_file_name);
                    if !cb4_buf_path.is_empty() {
                        bone_matrix_file_name = format!("{}-BoneMatrix.buf", name_prefix);
                        fs::copy(
                            &cb4_buf_path,
                            game_type_output_path.join(&bone_matrix_file_name),
                        )
                        .map_err(|e| format!("Failed to copy bone matrix: {}", e))?;
                    }
                }

                // Build VGMap from bone matrix (map local bone index -> merged global index)
                let mut vg_map: HashMap<String, i32> = HashMap::new();
                if !bone_matrix_file_name.is_empty() && submesh_vg_count > 0 {
                    let bone_matrix_buf_path = game_type_output_path.join(&bone_matrix_file_name);
                    let bone_matrix_buf = BoneMatrixBufFile::from_file(&bone_matrix_buf_path)?;
                    for vg_id in 0..submesh_vg_count {
                        let matrix_bytes =
                            bone_matrix_buf.get_buf_data_by_blend_index(vg_id as usize);
                        if matrix_bytes.iter().all(|&b| b == 0) {
                            continue;
                        }
                        let merge_key = Self::bone_matrix_merge_key(&matrix_bytes);
                        let merged_vg_id =
                            if let Some(&existing) = merged_bone_matrix_map.get(&merge_key) {
                                existing
                            } else {
                                let new_id = merged_vg_offset + vg_id;
                                merged_bone_matrix_map.insert(merge_key, new_id);
                                new_id
                            };
                        vg_map.insert(vg_id.to_string(), merged_vg_id);
                    }
                }

                // --- build SubMeshJson ---
                let mut submesh_json = SubMeshJson::new();
                submesh_json.game_preset = "NTEMI".to_string();
                submesh_json.vertex_limit_vb = vb0_hash;
                submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
                submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
                submesh_json.vertex_offset = vertex_offset;
                submesh_json.vertex_count = vertex_count;
                submesh_json.index_offset = index_offset;
                submesh_json.index_count = index_count;
                submesh_json.vg_count = submesh_vg_count;
                submesh_json.vg_offset = merged_vg_offset;
                submesh_json.vg_map = vg_map;
                submesh_json.cb4_hash = cb4_hash;
                submesh_json.bone_matrix_file_name = bone_matrix_file_name;

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
                    let buffer_type = if category_name == "Blend" {
                        "BlendWeight"
                    } else if category_name == "Normal" {
                        "TangentFrame"
                    } else {
                        "Normal"
                    };
                    submesh_json
                        .category_buffer_list
                        .push(SubMeshCategoryBuffer {
                            file_name: format!("{}-{}.buf", name_prefix, category_name),
                            buffer_type: buffer_type.to_string(),
                            d3d11_element_list: self.build_submesh_elements_for_category(
                                d3d11_game_type,
                                category_name,
                            ),
                        });
                }

                // Extra metadata: IB hash, draw call index, vertex range
                let ib_hash = SSMTStringUtils::get_file_hash_from_file_name(&ib_buf_file_name);
                submesh_json
                    .category_hash_dict
                    .insert("IB".to_string(), ib_hash);
                submesh_json
                    .category_hash_dict
                    .insert("DrawCallIndex".to_string(), ib_txt_file.index.clone());
                submesh_json
                    .category_hash_dict
                    .insert("VertexMin".to_string(), ib_buf_file.min_number.to_string());
                submesh_json
                    .category_hash_dict
                    .insert("VertexMax".to_string(), ib_buf_file.max_number.to_string());
                submesh_json.category_hash_dict.insert(
                    "VertexUnique".to_string(),
                    ib_buf_file.unique_vertex_count.to_string(),
                );

                // Texture bindings
                submesh_json.texture_mark_up_info_list = texture_mark_list.clone();

                // ShapeKeys (only for GPU pre-skinning)
                if d3d11_game_type.gpu_pre_skinning && shape_keys_info.vertex_count > 0 {
                    submesh_json.shape_keys_info = shape_keys_info.clone();
                }

                submesh_json
                    .save_to_file(game_type_output_path.join(format!("{}.json", name_prefix)))?;

                merged_vg_offset += submesh_vg_count;
            }
        }

        Ok(true)
    }

    fn get_hash_from_dump_filename(file_name: &str, marker: &str) -> String {
        file_name
            .split_once(marker)
            .and_then(|(_, suffix)| suffix.split('-').next())
            .unwrap_or_default()
            .to_string()
    }

    fn get_pointlist_index_by_draw_ib(
        &self,
        draw_ib: &str,
        trianglelist_index_list: &[String],
    ) -> String {
        let first_trianglelist_index = trianglelist_index_list.first().cloned().unwrap_or_default();
        if first_trianglelist_index.is_empty() {
            return String::new();
        }

        let trianglelist_vb0_filename = self
            .fa
            .data
            .filter_first_file(&format!("{}-vb0", first_trianglelist_index), ".buf")
            .unwrap_or_default();
        if trianglelist_vb0_filename.is_empty() {
            crate::extract_log!(
                "NTEMI PointlistIndex 识别失败: DrawIB {} 在 TrianglelistIndex {} 上未找到 vb0 文件",
                draw_ib, first_trianglelist_index
            );
            return String::new();
        }

        let vb0_hash = Self::get_hash_from_dump_filename(&trianglelist_vb0_filename, "vb0=");
        if vb0_hash.is_empty() {
            crate::extract_log!(
                "NTEMI PointlistIndex 识别失败: DrawIB {} 的 vb0 文件名无法解析 hash: {}",
                draw_ib,
                trianglelist_vb0_filename
            );
            return String::new();
        }

        let trianglelist_index_number = match first_trianglelist_index.parse::<i32>() {
            Ok(value) => value,
            Err(_) => return String::new(),
        };

        let hash_marker = format!("={}-", vb0_hash);
        let mut matched_index_list: Vec<i32> = Vec::new();

        for log_line in self.fa.log.lines.iter() {
            if !log_line.starts_with("00") {
                continue;
            }

            let current_index = log_line.get(0..6).unwrap_or_default();
            let current_index_number = match current_index.parse::<i32>() {
                Ok(value) => value,
                Err(_) => continue,
            };

            if current_index_number >= trianglelist_index_number {
                break;
            }

            if !log_line.contains("3DMigoto Dumping Buffer") {
                continue;
            }

            if log_line.contains(&hash_marker) {
                if !matched_index_list.contains(&current_index_number) {
                    matched_index_list.push(current_index_number);
                }
            }
        }

        matched_index_list.sort_unstable();

        if let Some(first_index) = matched_index_list.first() {
            let pointlist_index = format!("{:06}", first_index);
            crate::extract_log!(
                "NTEMI PointlistIndex 识别成功: DrawIB {} -> PointlistIndex {} (TrianglelistIndex {} / vb0 hash {} / matched {:?})",
                draw_ib, pointlist_index, first_trianglelist_index, vb0_hash, matched_index_list
            );
            return pointlist_index;
        }

        crate::extract_log!(
            "NTEMI PointlistIndex 识别失败: DrawIB {} 未能通过 vb0 hash {} 在 TrianglelistIndex {} 之前找到 Dumping Buffer 索引",
            draw_ib, vb0_hash, first_trianglelist_index
        );

        String::new()
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
        let current_gametype_folder_path = gametype_folder_path.join("NTEMI");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        let draw_ib_list: Vec<String> = if specify_drawib_extract {
            drawib_config
                .entries
                .iter()
                .map(|entry| entry.draw_ib.trim().to_string())
                .filter(|draw_ib| !draw_ib.is_empty())
                .collect()
        } else {
            let mut draw_ib_set: BTreeSet<String> = BTreeSet::new();
            for ib_txt_file_name in fa.data.filter_filelist("-ib=", ".txt") {
                let draw_ib = ib_txt_file_name.get(10..18).unwrap_or_default().to_string();
                if draw_ib.len() == 8 {
                    draw_ib_set.insert(draw_ib);
                }
            }
            draw_ib_set.into_iter().collect()
        };

        Ok(Self {
            fa,
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract,
            draw_ib_list,
            d3d11_gametype_lv2,
        })
    }

    pub fn detect_gametype_list(
        &self,
        draw_ib: &str,
        pointlist_index: &str,
        trianglelist_index_list: &[String],
    ) -> Result<Vec<D3D11GameTypeWrapper>, String> {
        let mut possible_game_type_list: Vec<D3D11GameTypeWrapper> = Vec::new();

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

            crate::extract_log!("当前数据类型: {}", d3d11_game_type.game_type_name);
            if find_at_least_one_gpu_type && !d3d11_game_type.gpu_pre_skinning {
                crate::extract_log!(
                    "自动优化:已经找到了满足条件的GPU类型，所以这个CPU类型就不用判断了"
                );
                continue;
            }

            let matched_trianglelist_index = self
                .get_first_matching_trianglelist_index(d3d11_game_type, trianglelist_index_list);
            crate::extract_log!(
                "AllSlot Matched TrianglelistIndex: {}",
                matched_trianglelist_index
            );

            let mut category_buf_filename_dict: HashMap<String, String> = HashMap::new();

            let mut all_category_slot_found = true;
            for category_kv in d3d11_game_type.category_slot_dict.iter() {
                let category = category_kv.0;
                let category_slot = category_kv.1;

                let extract_index: String;
                let topology = d3d11_game_type
                    .category_topology_dict
                    .get(category)
                    .cloned()
                    .unwrap_or_default();
                if topology == "pointlist" {
                    extract_index = pointlist_index.to_string();
                } else {
                    extract_index = matched_trianglelist_index.clone();
                }

                crate::extract_log!("CategorySlot: {}", category_slot);
                let search_str = format!("{}-{}=", extract_index, category_slot);
                let category_buf_filename = self
                    .fa
                    .data
                    .filter_first_file(&search_str, ".buf")
                    .unwrap_or_default();
                if category_buf_filename.is_empty() {
                    crate::extract_log!("未找到当前CategorySlot对应文件: {}", category_slot);
                    all_category_slot_found = false;
                    break;
                }

                category_buf_filename_dict.insert(category.clone(), category_buf_filename.clone());
                crate::extract_log!(
                    "CategorySlot: {} ExtractBufFileName: {}",
                    category_slot,
                    category_buf_filename
                );
            }

            if !all_category_slot_found {
                crate::extract_log!("未找到全部CategorySlot对应文件，跳过此数据类型");
                continue;
            }

            let mut all_slot_match = true;
            let mut vertex_count: u64 = 0;

            for (category_name, category_slot) in d3d11_game_type.category_slot_dict.iter() {
                crate::extract_log!("CategoryName: {}", category_name);
                crate::extract_log!("CategorySlot: {}", category_slot);

                let category_stride = d3d11_game_type
                    .category_stride_dict
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);

                if category_stride == 0 {
                    all_slot_match = false;
                    crate::extract_log!("未在数据类型定义中找到当前Category的Stride，匹配失败");
                    break;
                }

                if !category_buf_filename_dict.contains_key(category_name) {
                    crate::extract_log!("未检测到当前CategorySlot文件，匹配失败");
                    all_slot_match = false;
                    break;
                }

                let category_buf_filename = category_buf_filename_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                crate::extract_log!(
                    "CategorySlot: {} CategoryBufFileName: {}",
                    category_slot,
                    category_buf_filename
                );

                let category_buf_filepath =
                    self.fa.log.get_deduped_filepath(&category_buf_filename);
                if category_buf_filepath.is_empty() {
                    crate::extract_log!("未找到当前CategorySlot文件路径，匹配失败");
                    all_slot_match = false;
                    break;
                }

                // 对于 NTEMI，这里的 txt 文件名直接由对应 buf 文件名换后缀得到。
                let category_txt_filename =
                    SSMTFileUtils::get_filename_with_new_extension(&category_buf_filename, "txt")?;
                let category_txt_filepath =
                    self.fa.log.get_deduped_filepath(&category_txt_filename);
                let slot_file_size = if category_txt_filename.is_empty()
                    || category_txt_filepath.is_empty()
                    || !Path::new(&category_txt_filepath).is_file()
                {
                    SSMTFileUtils::get_file_size(&category_buf_filepath)?
                } else if d3d11_game_type.gpu_pre_skinning {
                    SSMTFileUtils::get_file_size(&category_buf_filepath)?
                } else {
                    match SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_txt_filepath) {
                        Ok(size) => size,
                        Err(e) => {
                            crate::extract_log!(
                                    "NTEMI metadata read failed, fallback to buf size. CategorySlot: {} CategoryBufFileName: {} TxtPath: {} Reason: {}",
                                    category_slot, category_buf_filename, category_txt_filepath, e
                                );
                            SSMTFileUtils::get_file_size(&category_buf_filepath)?
                        }
                    }
                };

                if slot_file_size == 0 || slot_file_size % category_stride != 0 {
                    crate::extract_log!(
                        "当前槽位: {} 文件大小({})与步长({})不匹配，跳过此数据类型",
                        category_slot,
                        slot_file_size,
                        category_stride
                    );
                    all_slot_match = false;
                    break;
                }

                let slot_vertex_count = slot_file_size / category_stride;

                if vertex_count == 0 {
                    vertex_count = slot_vertex_count;
                } else if vertex_count != slot_vertex_count {
                    crate::extract_log!(
                        "VertexCount: {} SlotVertexCount: {}",
                        vertex_count,
                        slot_vertex_count
                    );
                    crate::extract_log!(
                        "当前槽位: {} 文件数据不符合当前数据类型要求，跳过此数据类型",
                        category_slot
                    );
                    all_slot_match = false;
                    break;
                } else {
                    vertex_count = slot_vertex_count;
                }
            }

            if all_slot_match {
                crate::extract_log!("识别到数据类型: {}", d3d11_game_type.game_type_name);
                possible_game_type_list.push(D3D11GameTypeWrapper::new(
                    d3d11_game_type.clone(),
                    pointlist_index,
                    matched_trianglelist_index,
                ));
            }

            if !find_at_least_one_gpu_type {
                for gt in possible_game_type_list.iter() {
                    if gt.d3d11gametype.gpu_pre_skinning {
                        find_at_least_one_gpu_type = true;
                        break;
                    }
                }
            }
        }

        if possible_game_type_list.is_empty() {
            crate::extract_log!("无法识别 DrawIB {} 对应的数据类型", draw_ib);
            return Ok(possible_game_type_list);
        }

        //对最终过滤掉结果再过滤一次，只保留总Stride为最大的那些，防止较少Slot数量的数据类型被匹配到
        // let all_gpu_type = possible_game_type_list
        //     .iter()
        //     .all(|game_type_wrapper| game_type_wrapper.d3d11gametype.gpu_pre_skinning);

        // if all_gpu_type {
        //     let max_stride = possible_game_type_list
        //         .iter()
        //         .map(|game_type_wrapper| game_type_wrapper.d3d11gametype.get_self_stride())
        //         .max()
        //         .unwrap_or(0);

        //     possible_game_type_list = possible_game_type_list
        //         .into_iter()
        //         .filter(|game_type_wrapper| {
        //             game_type_wrapper.d3d11gametype.get_self_stride() == max_stride
        //         })
        //         .collect();
        // }

        crate::extract_log!("All Matched GameType:");
        for game_type_wrapper in possible_game_type_list.iter() {
            crate::extract_log!(
                "{} => PointlistIndex: {} MatchedTrianglelistIndex: {}",
                game_type_wrapper.d3d11gametype.game_type_name,
                game_type_wrapper.pointlist_index,
                game_type_wrapper.matched_trianglelistindex
            );
        }

        Ok(possible_game_type_list)
    }

    /// NTEMI-specific version of texture sync that resolves
    /// `DrawIndexedInstancedIndirect` args so submesh folder names match
    /// (same logic as `collect_match_first_index_ib_map`).
    fn sync_deduped_textures_and_json(&self, drawib_config: &DrawIBConfig) -> Result<(), String> {
        let mut component_drawcall_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();

        for entry in drawib_config.entries.iter() {
            let draw_ib = entry.draw_ib.trim();
            if draw_ib.is_empty() {
                continue;
            }

            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(draw_ib);
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
                    .fa
                    .data
                    .filter_first_file_by_content_and_suffix(&search_key, ".txt")
                    .unwrap_or_default();
                if ib_txt_filename.is_empty() {
                    continue;
                }

                let ib_txt_filepath = self.fa.log.get_deduped_filepath(&ib_txt_filename);
                if ib_txt_filepath.is_empty() || !Path::new(&ib_txt_filepath).exists() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_filepath, true)?;

                // Same indirect-resolve logic as collect_match_first_index_ib_map
                let (effective_first_index, effective_index_count_str) =
                    if ib_txt_file.first_index == "0" && ib_txt_file.index_count == "0" {
                        self.fa
                            .log
                            .resolve_indirect_draw_params(trianglelist_index)
                            .map(|(si, ic)| (si, ic.to_string()))
                            .unwrap_or((0, ib_txt_file.index_number_count.to_string()))
                    } else {
                        let fi = ib_txt_file.first_index.trim().parse::<u64>().unwrap_or(0);
                        (fi, ib_txt_file.index_number_count.to_string())
                    };

                first_index_ib_txt_map.insert(effective_first_index, ib_txt_filename);
                first_index_index_count_map
                    .insert(effective_first_index, effective_index_count_str);
                push_unique_string(
                    first_index_trianglelist_index_map
                        .entry(effective_first_index)
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

        // --- copy deduped textures and build index JSON ---
        let workspace_path = &self.workspace_path;
        let deduped_folder_path = PathBuf::from(workspace_path).join("DedupedTextures");
        SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path)?;

        if !component_drawcall_index_list_dict.is_empty() {
            for index_list in component_drawcall_index_list_dict.values() {
                for trianglelist_index in index_list.iter() {
                    let content_str = format!("{}-ps-t", trianglelist_index);
                    let ps_texture_all_filename_list =
                        self.fa.data.filter_texture_filename_list(&content_str);

                    for ps_texture_filename in ps_texture_all_filename_list {
                        let deduped_filepath =
                            self.fa.log.get_deduped_filepath(&ps_texture_filename);
                        if deduped_filepath.is_empty() {
                            continue;
                        }

                        let deduped_filename =
                            self.fa.log.get_deduped_filename(&ps_texture_filename);
                        let texture_unique_hash =
                            SSMTStringUtils::get_file_hash_from_file_name(&ps_texture_filename);

                        let target_texture_path = deduped_folder_path
                            .join(format!("{}_{}", texture_unique_hash, deduped_filename));
                        let target_texture_path_str =
                            target_texture_path.to_string_lossy().to_string();
                        let _ = SSMTFileUtils::copy_to_file_if_not_exists(
                            &deduped_filepath,
                            &target_texture_path_str,
                        );
                    }
                }
            }

            let component_json_path = crate::helper::mark_texture_helper::
                get_workspace_component_name_draw_call_index_list_json_path(workspace_path)?;
            let component_json =
                crate::helper::mark_texture_helper::ComponentNameDrawCallIndexListJson::from_map(
                    component_drawcall_index_list_dict.clone(),
                );
            if let Err(e) = component_json.save_to_file(&component_json_path) {
                crate::extract_error!("Warning: {}", e);
            }
        }

        let trianglelist_deduped_filename_json_path =
            crate::helper::mark_texture_helper::get_workspace_trianglelist_deduped_filename_json_path(
                workspace_path,
            )?;

        // Build trianglelist → deduped texture property map
        let mut trianglelist_deduped_map: HashMap<
            String,
            crate::helper::mark_texture_helper::TrianglelistDedupedTextureProperty,
        > = HashMap::new();
        let mut all_texture_names: Vec<String> = Vec::new();
        for index_list in component_drawcall_index_list_dict.values() {
            for trianglelist_index in index_list.iter() {
                let content_str = format!("{}-ps-t", trianglelist_index);
                let tex_list = self.fa.data.filter_texture_filename_list(&content_str);
                all_texture_names.extend(tex_list);
            }
        }
        for tex_name in all_texture_names {
            let hash = SSMTStringUtils::get_file_hash_from_file_name(&tex_name);
            let fa_log_deduped = self.fa.log.get_deduped_filename(&tex_name);
            let fa_data_deduped = String::new(); // simplified: lookup via hash in root
            let fa_log_deduped_file_name = if fa_log_deduped.trim().is_empty() {
                String::new()
            } else {
                format!("{}_{}", hash, fa_log_deduped)
            };
            trianglelist_deduped_map.insert(
                tex_name,
                crate::helper::mark_texture_helper::TrianglelistDedupedTextureProperty {
                    fa_log_deduped_file_name: fa_log_deduped_file_name.clone(),
                    fa_data_deduped_file_name: fa_data_deduped,
                    format: SSMTStringUtils::extract_texture_format_from_deduped_file_name(
                        &fa_log_deduped_file_name,
                    ),
                },
            );
        }
        let trianglelist_deduped_json =
            crate::helper::mark_texture_helper::TrianglelistDedupedFileNameJson::from_map(
                trianglelist_deduped_map,
            );
        if let Err(e) =
            trianglelist_deduped_json.save_to_file(&trianglelist_deduped_filename_json_path)
        {
            crate::extract_error!("Warning: {}", e);
        }

        Ok(())
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        crate::extract_log!("开始提取:");

        for draw_ib in self.draw_ib_list.iter() {
            //获取到当前DrawIB进行处理
            crate::extract_log!("当前DrawIB: {}", draw_ib);

            //获取TrianglelistIndex列表
            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(&draw_ib);
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TrianglelistIndex: {}", trianglelist_index);
            }

            //根据FrameAnalysis下面的log.txt，找到PointlistIndex
            let pointlist_index =
                self.get_pointlist_index_by_draw_ib(draw_ib, &trianglelist_index_list);

            if pointlist_index.is_empty() {
                print!("未找到对应的PointlistIndex，该DrawIB对应数据类型可能为CPU-PreSkinning类型")
            } else {
                crate::extract_log!("PointlistIndex: {:?}", pointlist_index);
            }

            //根据DrawIB和TrianglelistIndex列表，获取可能的数据类型列表
            let mut possible_gametype_list =
                self.detect_gametype_list(&draw_ib, &pointlist_index, &trianglelist_index_list)?;

            possible_gametype_list
                .retain(|gt| data_type_filter.allows(gt.d3d11gametype.gpu_pre_skinning));

            if possible_gametype_list.is_empty() {
                for x in trianglelist_index_list.iter() {
                    crate::extract_log!("Unrecognized TrianglelistIndex: {}", x);
                }

                crate::extract_new::log_skipped_drawib(
                    draw_ib,
                    format!(
                        "no data type matched. TrianglelistIndex: {:?} PointlistIndex: {}",
                        trianglelist_index_list, pointlist_index
                    ),
                );
                continue;
            }

            self.export_unity_vs_submeshes(
                draw_ib,
                &trianglelist_index_list,
                &possible_gametype_list,
            )?;
        }

        crate::extract_log!("提取正常执行完成");
        if self.specify_drawib_extract {
            self.sync_deduped_textures_and_json(&self.drawib_config)?;
        } else {
            let full_drawib_config = DrawIBConfig {
                path: String::new(),
                entries: self
                    .draw_ib_list
                    .clone()
                    .into_iter()
                    .map(|draw_ib| DrawIBEntry {
                        draw_ib: draw_ib.clone(),
                        alias: draw_ib,
                    })
                    .collect(),
            };
            self.sync_deduped_textures_and_json(&full_drawib_config)?;
        }
        Ok(())
    }
}
