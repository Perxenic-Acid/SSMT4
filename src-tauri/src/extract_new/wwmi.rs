use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::common::bone_matrix_buf_file::BoneMatrixBufFile;
use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::frame_analysis::frameanalysis_base::FrameAnalysisBase;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::common::shape_keys::ShapeKeys;
use crate::config::drawib_config::{DrawIBConfig, DrawIBEntry};
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::helper::workspace_texture_sync::sync_workspace_deduped_textures_and_json;
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshD3D11Element, SubMeshIndexBuffer, SubMeshJson,
};

pub struct WWMINewExtractor {
    base: FrameAnalysisBase,
}

impl WWMINewExtractor {
    pub fn new(
        frame_analysis_folder: &String,
        workspace_path: &String,
        is_full_extract: bool,
    ) -> Result<Self, String> {
        let base = FrameAnalysisBase::new(
            frame_analysis_folder,
            workspace_path,
            !is_full_extract,
            "WWMI",
        )?;

        Ok(Self { base })
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

    fn export_category_buffer(
        &self,
        category_name: &str,
        category_buf_filename: &str,
        category_output_buf_file_path: &Path,
    ) -> Result<(), String> {
        let category_buf_file_path = self.base.fa.log.get_deduped_filepath(category_buf_filename);
        if category_buf_file_path.is_empty() {
            return Err(format!(
                "Category {} deduped path is empty: {}",
                category_name, category_buf_filename
            ));
        }

        fs::copy(&category_buf_file_path, category_output_buf_file_path).map_err(|e| {
            format!(
                "Failed to copy category buffer file for category {}: {}",
                category_name, e
            )
        })?;
        Ok(())
    }

    /// 读取 Blend category buffer，遍历指定顶点索引列表，收集 submesh 实际使用的全局 VG 列表。
    ///
    /// 收集此 submesh 所有顶点实际使用的 blend index 值（全局 cb4 骨骼索引），
    /// 去重后统计唯一骨骼数量。
    fn collect_submesh_vg_indices(
        &self,
        d3d11_game_type: &D3D11GameType,
        category_buf_file_name_dict: &HashMap<String, String>,
        vertex_indices: &[u32],
    ) -> Result<Vec<i32>, String> {
        let blend_stride = match d3d11_game_type.category_stride_dict.get("Blend") {
            Some(&s) if s > 0 => s as usize,
            _ => {
                // 没有 Blend category（如某些 CPU pre-skinning 类型），返回空列表
                return Ok(Vec::new());
            }
        };

        let blend_buf_file_name = category_buf_file_name_dict
            .get("Blend")
            .cloned()
            .unwrap_or_default();
        if blend_buf_file_name.is_empty() {
            return Ok(Vec::new());
        }

        let blend_buf_path = self.base.fa.log.get_deduped_filepath(&blend_buf_file_name);
        if blend_buf_path.is_empty() {
            crate::extract_log!("Blend buf deduped path is empty: {}", blend_buf_file_name);
            return Ok(Vec::new());
        }

        let blend_bytes = fs::read(&blend_buf_path)
            .map_err(|e| format!("Failed to read Blend buffer {}: {}", blend_buf_path, e))?;

        // 收集此 submesh 所有顶点实际使用的唯一 blend index（全局 cb4 骨骼索引）
        // Blend buffer 前 4 字节为 BLENDINDICES（4 x R8_UINT），offset = 0
        let mut unique_indices: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
        for &vertex_idx in vertex_indices {
            let byte_start = (vertex_idx as usize).saturating_mul(blend_stride);
            if byte_start + 4 <= blend_bytes.len() {
                for &b in &blend_bytes[byte_start..byte_start + 4] {
                    unique_indices.insert(b as i32);
                }
            }
        }

        let vg_indices: Vec<i32> = unique_indices.into_iter().collect();
        crate::extract_log!("VGCount: {}", vg_indices.len());
        Ok(vg_indices)
    }

    fn get_match_first_index_ibtxt_filename_dict(
        &self,
        draw_ib: &str,
    ) -> Result<BTreeMap<u64, (String, String)>, String> {
        let mut out: BTreeMap<u64, (String, String)> = BTreeMap::new();
        let trianglelist_index_list = self.base.fa.data.get_trianglelist_index_list(draw_ib);

        for trianglelist_index in trianglelist_index_list {
            let ib_txt_file_name = self
                .base
                .fa
                .data
                .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                .unwrap_or_default();

            if ib_txt_file_name.is_empty() {
                continue;
            }

            let ib_txt_file_path = self.base.fa.log.get_deduped_filepath(&ib_txt_file_name);
            if ib_txt_file_path.is_empty() {
                continue;
            }

            let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, false)?;
            let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            out.insert(
                match_first_index,
                (ib_txt_file_name, trianglelist_index.clone()),
            );
        }

        Ok(out)
    }

    pub fn get_possible_gametype_list_unreal_vs(
        &self,
        draw_ib: &str,
        trianglelist_index_list: &[String],
    ) -> Result<Vec<D3D11GameType>, String> {
        let mut possible_game_type_list: Vec<D3D11GameType> = Vec::new();
        let mut find_at_least_one_gpu_type = false;

        for d3d11_game_type in self
            .base
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if find_at_least_one_gpu_type && !d3d11_game_type.gpu_pre_skinning {
                crate::extract_log!(
                    "自动优化:已经找到了满足条件的GPU类型，所以这个CPU类型就不用判断了"
                );
                continue;
            }

            crate::extract_log!("当前数据类型: {}", d3d11_game_type.game_type_name);

            let mut category_slot_file_name_dict: HashMap<String, String> = HashMap::new();
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TrianglelistIndex: {}", trianglelist_index);
                for category_slot in d3d11_game_type.category_slot_dict.values() {
                    crate::extract_log!("CategorySlot: {}", category_slot);

                    let category_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", trianglelist_index, category_slot),
                            ".buf",
                        )
                        .unwrap_or_default();

                    if category_file_name.is_empty() {
                        crate::extract_log!("未找到当前CategorySlot对应文件: {}", category_slot);
                        continue;
                    }

                    category_slot_file_name_dict
                        .insert(category_slot.clone(), category_file_name.clone());
                    crate::extract_log!(
                        "CategorySlot: {} ExtractFileName: {}",
                        category_slot,
                        category_file_name
                    );
                }
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
                    break;
                }

                if !category_slot_file_name_dict.contains_key(category_slot) {
                    crate::extract_log!("未检测到当前CategorySlot文件，匹配失败");
                    all_slot_match = false;
                    break;
                }

                let category_slot_file_name = category_slot_file_name_dict
                    .get(category_slot)
                    .cloned()
                    .unwrap_or_default();
                let category_slot_file_path = self
                    .base
                    .fa
                    .log
                    .get_deduped_filepath(&category_slot_file_name);
                if category_slot_file_path.is_empty() {
                    all_slot_match = false;
                    break;
                }

                let category_slot_txt_file_name = trianglelist_index_list
                    .iter()
                    .find_map(|trianglelist_index| {
                        let txt_file_name = self
                            .base
                            .fa
                            .data
                            .filter_first_file(
                                &format!("{}-{}", trianglelist_index, category_slot),
                                ".txt",
                            )
                            .unwrap_or_default();
                        if txt_file_name.is_empty() {
                            None
                        } else {
                            Some(txt_file_name)
                        }
                    })
                    .unwrap_or_default();
                let category_slot_txt_file_path = self
                    .base
                    .fa
                    .log
                    .get_deduped_filepath(&category_slot_txt_file_name);
                let slot_file_size = if category_slot_txt_file_name.is_empty()
                    || category_slot_txt_file_path.is_empty()
                {
                    SSMTFileUtils::get_file_size(&category_slot_file_path)?
                } else {
                    SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_slot_txt_file_path)?
                };
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
            return Ok(possible_game_type_list);
        }

        let all_gpu_type = possible_game_type_list
            .iter()
            .all(|d3d11_game_type| d3d11_game_type.gpu_pre_skinning);

        if all_gpu_type {
            let max_stride = possible_game_type_list
                .iter()
                .map(|d3d11_game_type| d3d11_game_type.get_self_stride())
                .max()
                .unwrap_or(0);

            possible_game_type_list = possible_game_type_list
                .into_iter()
                .filter(|d3d11_game_type| d3d11_game_type.get_self_stride() == max_stride)
                .collect();
        }

        crate::extract_log!("All Matched GameType:");
        for d3d11_game_type in possible_game_type_list.iter() {
            crate::extract_log!("{}", d3d11_game_type.game_type_name);
        }

        Ok(possible_game_type_list)
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        crate::extract_log!("开始提取:");

        for draw_ib in self.base.draw_ib_list.iter() {
            if draw_ib == "8d45cfee" {
                crate::extract_new::log_skipped_drawib(draw_ib, "known fake DrawIB");
                continue;
            } else {
                crate::extract_log!("当前DrawIB: {}", draw_ib);
            }

            let trianglelist_index_list = self.base.fa.data.get_trianglelist_index_list(&draw_ib);
            if trianglelist_index_list.is_empty() {
                crate::extract_new::log_skipped_drawib(draw_ib, "no trianglelist data files found");
                continue;
            }

            let mut total_category_slot_hash_dict: HashMap<String, String> = HashMap::new();
            let mut max_slot_number: usize = 0;
            let mut max_slot_trianglelist_index = String::new();

            crate::extract_log!("初始化 Total_CategorySlot_Hash_Dict:");
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("{}", trianglelist_index);
                let category_slot_hash_dict = self
                    .base
                    .fa
                    .log
                    .get_vb_category_hash_map_from_ia_set_vertex_buffer_by_index(
                        trianglelist_index,
                    );

                for (k, v) in category_slot_hash_dict.iter() {
                    total_category_slot_hash_dict.insert(k.clone(), v.clone());
                }

                if category_slot_hash_dict.len() >= max_slot_number {
                    max_slot_number = category_slot_hash_dict.len();
                    max_slot_trianglelist_index = trianglelist_index.clone();
                }
            }

            crate::extract_log!("TrianglelistIndex: {}", max_slot_trianglelist_index);
            crate::extract_log!("MaxSlotNumber: {}", max_slot_number);
            for (slot, hash) in total_category_slot_hash_dict.iter() {
                crate::extract_log!("CategorySlot: {} Hash: {}", slot, hash);
            }

            let mut possible_d3d11_game_type_list =
                self.get_possible_gametype_list_unreal_vs(&draw_ib, &trianglelist_index_list)?;

            possible_d3d11_game_type_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));

            //数据类型不能为空
            if possible_d3d11_game_type_list.is_empty() {
                crate::extract_new::log_skipped_drawib(
                    draw_ib,
                    format!(
                        "no data type matched. TrianglelistIndex: {:?}",
                        trianglelist_index_list
                    ),
                );
                continue;
            }

            //输出每一种数据类型
            for d3d11_game_type in possible_d3d11_game_type_list.iter() {
                crate::extract_log!("输出数据类型: {}", d3d11_game_type.game_type_name);
                let mut merged_vg_offset = 0i32;
                let mut merged_bone_matrix_map: HashMap<String, i32> = HashMap::new();

                //得到每个分类对应buf文件
                let mut category_buf_file_name_dict: HashMap<String, String> = HashMap::new();
                for (category_name, category_slot) in d3d11_game_type.category_slot_dict.iter() {
                    let category_buf_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", max_slot_trianglelist_index, category_slot),
                            ".buf",
                        )
                        .unwrap_or_default();
                    category_buf_file_name_dict
                        .insert(category_name.clone(), category_buf_file_name);
                }

                if let Some(vb0_hash) = total_category_slot_hash_dict.get("vb0") {
                    crate::extract_log!("Metadata.json vb0_hash: {}", vb0_hash);
                } else {
                    crate::extract_log!("Can't get vb0_hash for Metadata.json");
                }

                let cb4_file_name = self
                    .base
                    .fa
                    .data
                    .filter_first_file(&format!("{}-vs-cb4=", max_slot_trianglelist_index), ".buf")
                    .unwrap_or_default();
                let mut cb4_hash = String::new();
                if !cb4_file_name.is_empty() {
                    cb4_hash = cb4_file_name.chars().skip(14).take(8).collect();
                    crate::extract_log!("Metadata.json cb4_hash: {}", cb4_hash);
                } else {
                    crate::extract_log!("Can't get cb4_hash for Metadata.json");
                }

                let mut shapekey_extract_index = String::new();
                let mut shape_key_offset_buf_file_name = String::new();
                let mut shape_key_vertex_id_buf_file_name = String::new();
                let mut shape_key_vertex_offset_buf_file_name = String::new();
                let mut shape_key_scale_buf_file_name = String::new();

                let mut shapekeys = ShapeKeys::default();
                if let Some(shapekey_hash) = total_category_slot_hash_dict.get("vb6") {
                    shapekeys.offsets_hash = shapekey_hash.clone();
                    let shapekey_index_list = self
                        .base
                        .fa
                        .log
                        .get_drawcall_index_list_by_hash(shapekey_hash, false);

                    for index in shapekey_index_list.iter() {
                        let shape_key_category_slot_hash_map = self
                            .base
                            .fa
                            .log
                            .get_compute_shader_slot_hash_map_from_csset_shader_resources_by_index(
                                index,
                            );
                        if shape_key_category_slot_hash_map.contains_key("cs-t0")
                            && shape_key_category_slot_hash_map.contains_key("cs-t1")
                        {
                            shapekey_extract_index = index.clone();
                        }
                    }
                }

                if !shapekey_extract_index.is_empty() {
                    shape_key_offset_buf_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(&format!("{}-cs-cb0", shapekey_extract_index), ".buf")
                        .unwrap_or_default();
                    shape_key_vertex_id_buf_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(&format!("{}-cs-t0", shapekey_extract_index), ".buf")
                        .unwrap_or_default();
                    shape_key_vertex_offset_buf_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(&format!("{}-cs-t1", shapekey_extract_index), ".buf")
                        .unwrap_or_default();

                    if !shape_key_offset_buf_file_name.is_empty() {
                        // 解析 cs-cb0 buffer：前 128 个 u32 为 shapekey_offsets 数组
                        // vertex_count = offsets[127]（末位偏移 = 受影响顶点总数）
                        // checksum = offsets[0]+[1]+[2]+[3]
                        // dispatch_y = ceil(vertex_count / 32)
                        let cb0_file_path = self
                            .base
                            .fa
                            .log
                            .get_deduped_filepath(&shape_key_offset_buf_file_name);
                        if !cb0_file_path.is_empty() {
                            match SSMTBinaryUtils::read_as_r32_uint(&cb0_file_path) {
                                Ok(cb0_u32) if cb0_u32.len() >= 128 => {
                                    let sk_vertex_count = cb0_u32[127] as i32;
                                    let sk_checksum = (cb0_u32[0]
                                        .wrapping_add(cb0_u32[1])
                                        .wrapping_add(cb0_u32[2])
                                        .wrapping_add(cb0_u32[3]))
                                        as i32;
                                    let sk_dispatch_y = (sk_vertex_count + 31) / 32;
                                    shapekeys.vertex_count = sk_vertex_count;
                                    shapekeys.checksum = sk_checksum;
                                    shapekeys.dispatch_y = sk_dispatch_y;
                                    crate::extract_log!(
                                        "ShapeKeys: vertex_count={} dispatch_y={} checksum={}",
                                        sk_vertex_count,
                                        sk_dispatch_y,
                                        sk_checksum
                                    );
                                }
                                Ok(_) => {
                                    crate::extract_log!("cs-cb0 buffer 太短，无法读取 128 个 u32")
                                }
                                Err(e) => crate::extract_log!("读取 cs-cb0 buffer 失败: {}", e),
                            }
                        }
                    }

                    if !shape_key_vertex_id_buf_file_name.is_empty() {
                        shapekeys.vertex_ids_hash = self.build_slot_hash_from_buf_file_name(
                            "ShapeKeyVertexId",
                            "cs-t0",
                            &shape_key_vertex_id_buf_file_name,
                        )?;
                    }

                    if !shape_key_vertex_offset_buf_file_name.is_empty() {
                        shapekeys.vertex_offsets_hash = self.build_slot_hash_from_buf_file_name(
                            "ShapeKeyVertexOffset",
                            "cs-t1",
                            &shape_key_vertex_offset_buf_file_name,
                        )?;
                    }

                    shape_key_scale_buf_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(&format!("{}-u1=", shapekey_extract_index), ".buf")
                        .unwrap_or_default();
                    if !shape_key_scale_buf_file_name.is_empty() {
                        shapekeys.scale_hash = shape_key_scale_buf_file_name
                            .chars()
                            .skip(10)
                            .take(8)
                            .collect();
                    }
                } else {
                    crate::extract_log!(
                        "存在VB6槽位，但无法找到ShapeKey提取Index，跳过形态键提取。"
                    );
                }

                //逐个Submesh输出内容

                let unique_ib_txt_filename_dict =
                    self.get_match_first_index_ibtxt_filename_dict(&draw_ib)?;

                for (match_first_index, (ib_txt_filename, trianglelist_index)) in
                    unique_ib_txt_filename_dict.iter()
                {
                    crate::extract_log!(
                        "MatchFirstIndex: {} IBTxtFileName: {} TrianglelistIndex: {}",
                        match_first_index,
                        ib_txt_filename,
                        trianglelist_index
                    );

                    let unique_ib_txt_file_path =
                        self.base.fa.log.get_deduped_filepath(ib_txt_filename);
                    if unique_ib_txt_file_path.is_empty() {
                        continue;
                    }
                    let unique_ib_txt_file =
                        IndexBufferTxtFile::new(&unique_ib_txt_file_path, true)?;
                    let ib_file_format = if unique_ib_txt_file.format == "DXGI_FORMAT_R32_UINT" {
                        "DXGI_FORMAT_R32_UINT".to_string()
                    } else {
                        "DXGI_FORMAT_R16_UINT".to_string()
                    };
                    let ib_buf_file_name =
                        SSMTFileUtils::get_filename_with_new_extension(ib_txt_filename, "buf")?;
                    let ib_buf_file_path = self.base.fa.log.get_deduped_filepath(&ib_buf_file_name);
                    if ib_buf_file_path.is_empty() {
                        crate::extract_log!("IB buf deduped path is empty: {}", ib_buf_file_name);
                        continue;
                    }
                    let unique_str_folder_name = format!(
                        "{}-{}-{}",
                        draw_ib, unique_ib_txt_file.index_count, unique_ib_txt_file.first_index
                    );

                    let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);

                    let game_type_output_path = PathBuf::from(&self.base.workspace_path)
                        .join(&unique_str_folder_name)
                        .join(&game_type_folder_name);
                    SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;

                    let index_count: usize =
                        unique_ib_txt_file.index_count.parse::<usize>().unwrap_or(0);

                    let mut divide_ib_buf_file =
                        IndexBufferBufFile::from_file(&ib_buf_file_path, &ib_file_format)?;
                    divide_ib_buf_file.self_divide(*match_first_index as usize, index_count);

                    let output_ib_buf_file_path =
                        game_type_output_path.join(format!("{}.ib", unique_str_folder_name));
                    divide_ib_buf_file.save_to_file_uint32(&output_ib_buf_file_path, 0)?;

                    // 读取此 submesh 实际使用的 local VG 列表，再根据 BoneMatrix 内容构建 merged VG 语义
                    let submesh_vg_indices = self.collect_submesh_vg_indices(
                        d3d11_game_type,
                        &category_buf_file_name_dict,
                        &divide_ib_buf_file.number_list,
                    )?;
                    // vg_count 使用稠密范围 (max blend index + 1)，与 WWMI-Tools 参考实现一致
                    let submesh_vg_count: i32 = submesh_vg_indices.last().map_or(0, |&m| m + 1);

                    for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                        let category_buf_file_name = category_buf_file_name_dict
                            .get(category_name)
                            .cloned()
                            .unwrap_or_default();
                        let category_output_buf_file_path = game_type_output_path
                            .join(format!("{}-{}.buf", unique_str_folder_name, category_name));
                        self.export_category_buffer(
                            category_name,
                            &category_buf_file_name,
                            &category_output_buf_file_path,
                        )?;
                    }

                    if !shape_key_offset_buf_file_name.is_empty() {
                        let source_file_path = self
                            .base
                            .fa
                            .log
                            .get_deduped_filepath(&shape_key_offset_buf_file_name);
                        if source_file_path.is_empty() {
                            crate::extract_log!(
                                "ShapeKeyOffset deduped path is empty: {}",
                                shape_key_offset_buf_file_name
                            );
                        } else {
                            fs::copy(
                                &source_file_path,
                                game_type_output_path
                                    .join(format!("{}-ShapeKeyOffset.buf", unique_str_folder_name)),
                            )
                            .map_err(|e| {
                                format!("Failed to copy ShapeKeyOffset buffer file: {}", e)
                            })?;
                        }
                    }

                    if !shape_key_vertex_id_buf_file_name.is_empty() {
                        let source_file_path = self
                            .base
                            .fa
                            .log
                            .get_deduped_filepath(&shape_key_vertex_id_buf_file_name);
                        if source_file_path.is_empty() {
                            crate::extract_log!(
                                "ShapeKeyVertexId deduped path is empty: {}",
                                shape_key_vertex_id_buf_file_name
                            );
                        } else {
                            fs::copy(
                                &source_file_path,
                                game_type_output_path.join(format!(
                                    "{}-ShapeKeyVertexId.buf",
                                    unique_str_folder_name
                                )),
                            )
                            .map_err(|e| {
                                format!("Failed to copy ShapeKeyVertexId buffer file: {}", e)
                            })?;
                        }
                    }

                    if !shape_key_vertex_offset_buf_file_name.is_empty() {
                        let source_file_path = self
                            .base
                            .fa
                            .log
                            .get_deduped_filepath(&shape_key_vertex_offset_buf_file_name);
                        if source_file_path.is_empty() {
                            crate::extract_log!(
                                "ShapeKeyVertexOffset deduped path is empty: {}",
                                shape_key_vertex_offset_buf_file_name
                            );
                        } else {
                            fs::copy(
                                &source_file_path,
                                game_type_output_path.join(format!(
                                    "{}-ShapeKeyVertexOffset.buf",
                                    unique_str_folder_name
                                )),
                            )
                            .map_err(|e| {
                                format!("Failed to copy ShapeKeyVertexOffset buffer file: {}", e)
                            })?;
                        }
                    }

                    if !shape_key_scale_buf_file_name.is_empty() {
                        let source_file_path = self
                            .base
                            .fa
                            .log
                            .get_deduped_filepath(&shape_key_scale_buf_file_name);
                        if source_file_path.is_empty() {
                            crate::extract_log!(
                                "ShapeKeyScale deduped path is empty: {}",
                                shape_key_scale_buf_file_name
                            );
                        } else {
                            fs::copy(
                                &source_file_path,
                                game_type_output_path
                                    .join(format!("{}-ShapeKeyScale.buf", unique_str_folder_name)),
                            )
                            .map_err(|e| {
                                format!("Failed to copy ShapeKeyScale buffer file: {}", e)
                            })?;
                        }
                    }

                    // 每个 submesh 使用自身 trianglelist_index 对应的 cb4 骨骼矩阵（与 WWMI-Tools 一致）
                    let mut bone_matrix_file_name = String::new();
                    let submesh_cb4_file_name = self
                        .base
                        .fa
                        .data
                        .filter_first_file(&format!("{}-vs-cb4=", trianglelist_index), ".buf")
                        .unwrap_or_default();
                    if !submesh_cb4_file_name.is_empty() {
                        let cb4_buf_path = self
                            .base
                            .fa
                            .log
                            .get_deduped_filepath(&submesh_cb4_file_name);
                        if cb4_buf_path.is_empty() {
                            crate::extract_log!(
                                "cb4 deduped path is empty: {}",
                                submesh_cb4_file_name
                            );
                        } else {
                            let dest_name = format!("{}-BoneMatrix.buf", unique_str_folder_name);
                            fs::copy(&cb4_buf_path, game_type_output_path.join(&dest_name))
                                .map_err(|e| {
                                    format!("Failed to copy BoneMatrix buffer file: {}", e)
                                })?;
                            bone_matrix_file_name = dest_name;
                        }
                    }

                    let mut submesh_json = SubMeshJson::new();
                    submesh_json.game_preset = "WWMI".to_string();
                    submesh_json.vertex_limit_vb = total_category_slot_hash_dict
                        .get("vb0")
                        .cloned()
                        .unwrap_or_default();
                    submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
                    submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
                    submesh_json.cb4_hash = cb4_hash.clone();
                    submesh_json.bone_matrix_file_name = bone_matrix_file_name.clone();
                    // 顶点范围：IB 中出现的最小/唯一顶点信息
                    submesh_json.vertex_offset = divide_ib_buf_file.min_number as i64;
                    submesh_json.vertex_count = divide_ib_buf_file.unique_vertex_count as i64;
                    // IB 范围：在全量 IB 中的起始位置和元素数量
                    submesh_json.index_offset = *match_first_index as i64;
                    submesh_json.index_count = divide_ib_buf_file.number_count as i64;
                    // 顶点组信息：local blend index 通过 vg_offset/vg_map 还原到 merged 骨架语义
                    submesh_json.vg_count = submesh_vg_count;
                    submesh_json.vg_offset = merged_vg_offset;

                    if !bone_matrix_file_name.is_empty() && submesh_vg_count > 0 {
                        let bone_matrix_buf_path =
                            game_type_output_path.join(&bone_matrix_file_name);
                        let bone_matrix_buf = BoneMatrixBufFile::from_file(&bone_matrix_buf_path)?;

                        for vg_id in 0..submesh_vg_count {
                            let bone_matrix_value =
                                bone_matrix_buf.get_buf_data_by_blend_index(vg_id as usize);
                            if bone_matrix_value.iter().all(|&b| b == 0) {
                                continue;
                            }
                            let bone_matrix_key = bone_matrix_value
                                .iter()
                                .map(|byte| format!("{:02X}", byte))
                                .collect::<Vec<String>>()
                                .join("-");

                            let merged_vg_id = if let Some(existing_vg_id) =
                                merged_bone_matrix_map.get(&bone_matrix_key)
                            {
                                *existing_vg_id
                            } else {
                                let new_vg_id = merged_vg_offset + vg_id;
                                merged_bone_matrix_map.insert(bone_matrix_key, new_vg_id);
                                new_vg_id
                            };

                            submesh_json.vg_map.insert(vg_id.to_string(), merged_vg_id);
                        }
                    }
                    // ShapeKey 元数据
                    submesh_json.shape_keys_info = shapekeys.clone();
                    submesh_json.index_buffer_list.push(SubMeshIndexBuffer {
                        dxgi_format: "DXGI_FORMAT_R32_UINT".to_string(),
                        file_name: format!("{}.ib", unique_str_folder_name),
                    });

                    for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                        let category_buf_file_name = category_buf_file_name_dict
                            .get(category_name)
                            .cloned()
                            .unwrap_or_default();
                        let category_slot = d3d11_game_type
                            .category_slot_dict
                            .get(category_name)
                            .ok_or_else(|| {
                            format!("Category slot not found for category: {}", category_name)
                        })?;
                        let start_index = 8usize + category_slot.len();
                        let category_hash: String = category_buf_file_name
                            .chars()
                            .skip(start_index)
                            .take(8)
                            .collect();
                        if category_hash.len() != 8 {
                            return Err(format!(
                                "Cannot parse hash from buf file name: {} (category={}, slot={})",
                                category_buf_file_name, category_name, category_slot
                            ));
                        }
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
                                file_name: format!(
                                    "{}-{}.buf",
                                    unique_str_folder_name, category_name
                                ),
                                buffer_type: "Normal".to_string(),
                                d3d11_element_list: self.build_submesh_elements_for_category(
                                    d3d11_game_type,
                                    category_name,
                                ),
                            });
                    }

                    if !shape_key_offset_buf_file_name.is_empty() {
                        submesh_json.category_hash_dict.insert(
                            "ShapeKeyOffset".to_string(),
                            self.build_slot_hash_from_buf_file_name(
                                "ShapeKeyOffset",
                                "cs-cb0",
                                &shape_key_offset_buf_file_name,
                            )?,
                        );
                        submesh_json
                            .category_draw_category_map
                            .insert("ShapeKeyOffset".to_string(), "ShapeKeyOffset".to_string());
                        submesh_json
                            .category_buffer_list
                            .push(SubMeshCategoryBuffer {
                                file_name: format!("{}-ShapeKeyOffset.buf", unique_str_folder_name),
                                buffer_type: "ShapeKeyOffset".to_string(),
                                d3d11_element_list: Vec::new(),
                            });
                    }

                    if !shape_key_vertex_id_buf_file_name.is_empty() {
                        submesh_json.category_hash_dict.insert(
                            "ShapeKeyVertexId".to_string(),
                            self.build_slot_hash_from_buf_file_name(
                                "ShapeKeyVertexId",
                                "cs-t0",
                                &shape_key_vertex_id_buf_file_name,
                            )?,
                        );
                        submesh_json.category_draw_category_map.insert(
                            "ShapeKeyVertexId".to_string(),
                            "ShapeKeyVertexId".to_string(),
                        );
                        submesh_json
                            .category_buffer_list
                            .push(SubMeshCategoryBuffer {
                                file_name: format!(
                                    "{}-ShapeKeyVertexId.buf",
                                    unique_str_folder_name
                                ),
                                buffer_type: "ShapeKeyVertexId".to_string(),
                                d3d11_element_list: Vec::new(),
                            });
                    }

                    if !shape_key_vertex_offset_buf_file_name.is_empty() {
                        submesh_json.category_hash_dict.insert(
                            "ShapeKeyVertexOffset".to_string(),
                            self.build_slot_hash_from_buf_file_name(
                                "ShapeKeyVertexOffset",
                                "cs-t1",
                                &shape_key_vertex_offset_buf_file_name,
                            )?,
                        );
                        submesh_json.category_draw_category_map.insert(
                            "ShapeKeyVertexOffset".to_string(),
                            "ShapeKeyVertexOffset".to_string(),
                        );
                        submesh_json
                            .category_buffer_list
                            .push(SubMeshCategoryBuffer {
                                file_name: format!(
                                    "{}-ShapeKeyVertexOffset.buf",
                                    unique_str_folder_name
                                ),
                                buffer_type: "ShapeKeyVertexOffset".to_string(),
                                d3d11_element_list: Vec::new(),
                            });
                    }

                    submesh_json.save_to_file(
                        game_type_output_path.join(format!("{}.json", unique_str_folder_name)),
                    )?;

                    merged_vg_offset += submesh_vg_count;
                }
            }
        }

        crate::extract_log!("提取正常执行完成");
        if self.base.specify_drawib_extract {
            sync_workspace_deduped_textures_and_json(
                &self.base.fa,
                &self.base.drawib_config,
                &self.base.workspace_path,
                !self.base.specify_drawib_extract,
            )?;
        } else {
            let full_drawib_config = DrawIBConfig {
                path: String::new(),
                entries: self
                    .base
                    .draw_ib_list
                    .iter()
                    .cloned()
                    .map(|draw_ib| DrawIBEntry {
                        draw_ib: draw_ib.clone(),
                        alias: draw_ib,
                    })
                    .collect(),
            };
            sync_workspace_deduped_textures_and_json(
                &self.base.fa,
                &full_drawib_config,
                &self.base.workspace_path,
                !self.base.specify_drawib_extract,
            )?;
        }
        Ok(())
    }
}
