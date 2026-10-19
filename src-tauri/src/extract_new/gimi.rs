use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::common::index_buffer_buf_file::IndexBufferBufFile;
use crate::common::index_buffer_txt_file::IndexBufferTxtFile;
use crate::config::drawib_config::{DrawIBConfig, DrawIBEntry};
use crate::config::path_manager::PathManager;
use crate::extract_new::extract_services::FullExtractDataTypeFilter;
use crate::helper::workspace_texture_sync::sync_workspace_deduped_textures_and_json;
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::workspace::submesh_json::{
    SubMeshCategoryBuffer, SubMeshD3D11Element, SubMeshIndexBuffer, SubMeshJson,
};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

pub struct GIMINewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl GIMINewExtractor {
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
    ) -> Result<BTreeMap<u64, (String, Vec<String>)>, String> {
        let mut result: BTreeMap<u64, (String, Vec<String>)> = BTreeMap::new();
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
            let entry = result
                .entry(match_first_index)
                .or_insert_with(|| (ib_file_name.clone(), Vec::new()));
            // Keep the same representative IB filename that the old BTreeMap
            // insertion produced for a shared FirstIndex, while retaining all
            // matching draw-call indexes as optional metadata.
            entry.0 = ib_file_name;
            if !entry.1.contains(trianglelist_index) {
                entry.1.push(trianglelist_index.clone());
            }
        }
        for (_, draw_call_indexes) in result.values_mut() {
            draw_call_indexes.sort();
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
        for (match_first_index, (ib_file_name, _)) in &match_first_index_ib_txt_file_name_dict {
            crate::extract_log!(
                "MatchFirstIndex: {} IBFileName: {}",
                match_first_index,
                ib_file_name
            );
        }
        for d3d11_game_type in possible_d3d11_game_type_list {
            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);
            for (_match_first_index, (ib_txt_file_name, draw_call_indexes)) in
                &match_first_index_ib_txt_file_name_dict
            {
                // Extract the per-submesh draw index (first 6 chars of the IB file name)
                let per_ib_trianglelist_index: String = ib_txt_file_name.chars().take(6).collect();
                let vertex_limit_vb = self.build_vertex_limit_vb(&per_ib_trianglelist_index);
                let mut category_buf_filename_map: HashMap<String, String> = HashMap::new();
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
                    let filename_search_str = format!("{}-{}", extract_index, category_slot);
                    let category_buf_filename = self
                        .fa
                        .data
                        .filter_first_file(&filename_search_str, ".buf")?;
                    category_buf_filename_map.insert(category_name.clone(), category_buf_filename);
                }
                let ib_buf_file_name =
                    SSMTFileUtils::get_filename_with_new_extension(ib_txt_file_name, "buf")?;
                let ib_txt_file_path = self.fa.log.get_deduped_filepath(ib_txt_file_name);
                let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);
                if ib_txt_file_path.is_empty() || ib_buf_file_path.is_empty() {
                    continue;
                }
                let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, true)?;
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
                        d3d11_game_type.gpu_pre_skinning,
                        &category_output_buf_file_path,
                    )?;
                }
                let mut submesh_json = SubMeshJson::new();
                submesh_json.game_preset = game_preset.to_string();
                submesh_json.vertex_limit_vb = vertex_limit_vb.clone();
                submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
                submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
                // Do not populate VertexOffset/VertexCount for GIMI. Its submeshes
                // share global GPU vertex buffers and their referenced vertices are
                // not guaranteed to form a contiguous range. The Blender importer
                // treats non-zero values as a slicing instruction, so only the
                // draw-call identity is persisted here.
                submesh_json.draw_call_index_list = draw_call_indexes.clone();
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
                            d3d11_element_list: self.build_submesh_elements_for_category(
                                d3d11_game_type,
                                category_name,
                            ),
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
        let current_gametype_folder_path = gametype_folder_path.join("GIMI");
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
        let mut possible_gametype_list: Vec<D3D11GameType> = Vec::new();
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

            crate::extract_log!("当前数据类型: {}", d3d11_game_type.game_type_name);

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

            let mut category_buf_filesize_map: HashMap<String, u64> = HashMap::new();
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
                    "当前分类: {} 提取Index: {} 提取槽位: {}",
                    category_name,
                    extract_index,
                    category_slot
                );

                let search_key = format!("{}-{}", extract_index, category_slot);
                let category_buf_file_name = self
                    .fa
                    .data
                    .filter_first_file(&search_key, ".buf")
                    .unwrap_or_default();
                let category_buf_txt_file_name = self
                    .fa
                    .data
                    .filter_first_file(&search_key, ".txt")
                    .unwrap_or_default();

                crate::extract_log!("CategoryBufFileName: {}", category_buf_file_name);
                let category_buf_file_path =
                    match self.fa.log.resolve_dump_filepath(&category_buf_file_name) {
                        Ok(path) => path.to_string_lossy().into_owned(),
                        Err(error) => {
                            crate::extract_log!("Category {} 读取失败: {}", category_name, error);
                            all_file_exists = false;
                            break;
                        }
                    };
                crate::extract_log!(
                    "Category: {} File: {}",
                    category_name,
                    category_buf_file_path
                );

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
                crate::extract_log!("FileSize: {}", file_size);
                category_buf_filesize_map.insert(category_name.clone(), file_size);
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
                let file_size = category_buf_filesize_map
                    .get(category_name)
                    .copied()
                    .unwrap_or(0);

                let tmp_number = if category_stride > 0 {
                    file_size / category_stride
                } else {
                    0
                };

                if tmp_number == 0 {
                    crate::extract_log!("槽位的文件大小不能为0，槽位匹配失败，跳过此数据类型");
                    all_match = false;
                    break;
                }

                if !d3d11_game_type.gpu_pre_skinning {
                    let yu_shu = file_size % category_stride;
                    if yu_shu != 0 {
                        crate::extract_log!("余数不为0: {}，槽位匹配失败", yu_shu);
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

                    let search_key = format!("{}-{}", trianglelist_index, category_slot);
                    let category_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(&search_key, ".txt")
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
                        crate::extract_log!("槽位的txt文件路径不存在，跳过此数据类型。");
                        all_match = false;
                        break;
                    }

                    let vertex_count_txt = SSMTFileUtils::find_migoto_ini_attribute_in_file(
                        Path::new(&category_txt_file_path),
                        "vertex count",
                    )?;

                    if vertex_count_txt.trim().is_empty() {
                        let show_stride = SSMTFileUtils::find_migoto_ini_attribute_in_file(
                            Path::new(&category_txt_file_path),
                            "stride",
                        )?;

                        if !show_stride.trim().is_empty() {
                            let show_stride_count = show_stride.parse::<u64>().unwrap_or(0);
                            if show_stride_count != category_stride {
                                crate::extract_log!(
                                    "槽位的txt文件Stride与数据类型Stride不符，跳过此数据类型。"
                                );
                                all_match = false;
                                break;
                            }
                        }
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
                possible_gametype_list.push(d3d11_game_type.clone());
            }

            if !find_at_least_one_gpu_type {
                for gt in possible_gametype_list.iter() {
                    if gt.gpu_pre_skinning {
                        find_at_least_one_gpu_type = true;
                        break;
                    }
                }
            }
        }

        if possible_gametype_list.is_empty() {
            crate::extract_log!("无法识别 DrawIB {} 对应的数据类型", draw_ib);
        } else {
            crate::extract_log!("All Matched GameType:");
            for gt in possible_gametype_list.iter() {
                crate::extract_log!("{}", gt.game_type_name);
            }
        }

        Ok(possible_gametype_list)
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
            "GIMI",
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
        let draw_ib_list = if self.specify_drawib_extract {
            self.drawib_config
                .entries
                .iter()
                .map(|entry| entry.draw_ib.clone())
                .collect::<Vec<String>>()
        } else {
            self.fa.data.get_all_drawib_list()
        };

        for draw_ib in draw_ib_list.iter() {
            crate::extract_log!("DrawIB: {}", draw_ib);

            let pointlist_index = self
                .fa
                .log
                .get_last_pointlist_index_by_hash(&draw_ib)
                .unwrap_or_default();
            crate::extract_log!("PointlistIndex: {:?}", pointlist_index);

            if pointlist_index.is_empty() {
                crate::extract_log!("未找到对应PointlistIndex，该DrawIB可能为CPU-PreSkinning类型");
            }

            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(&draw_ib);
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TriangleListIndex: {}", trianglelist_index);
            }

            let extract_success = self.extract_fee307b98a965c16(
                &draw_ib,
                &pointlist_index,
                &trianglelist_index_list,
                data_type_filter,
            )?;

            if !extract_success {
                crate::extract_new::log_skipped_drawib(
                    draw_ib,
                    format!(
                        "no valid data type matched. PointlistIndex: {} TrianglelistIndex: {:?}",
                        pointlist_index, trianglelist_index_list
                    ),
                );
                continue;
            }
        }

        if self.specify_drawib_extract {
            sync_workspace_deduped_textures_and_json(
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
            sync_workspace_deduped_textures_and_json(
                &self.fa,
                &full_drawib_config,
                &self.workspace_path,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPU_TYPE: &str = "GPU_P12_N12_TA16_C4_T8_BW16_BI16_";

    fn fixture_extractor(folder: &Path) -> GIMINewExtractor {
        let types = [GPU_TYPE, "CPU_P12_N12_TA16_C4_T8_"]
            .into_iter()
            .map(|name| {
                D3D11GameType::from_json_file(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("resources/GameType/GIMI")
                        .join(format!("{name}.json")),
                )
                .unwrap()
            })
            .collect();
        GIMINewExtractor {
            fa: FrameAnalysis::new(folder.to_str().unwrap()).unwrap(),
            workspace_path: String::new(),
            drawib_config: DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            },
            specify_drawib_extract: false,
            d3d11_gametype_lv2: D3D11GameTypeLv2 {
                ordered_gpu_cpu_d3d11_gametype_list: types,
                ..Default::default()
            },
        }
    }

    fn assert_gpu_match(extractor: &GIMINewExtractor) {
        for index in ["000073", "000138"] {
            let matched = extractor
                .get_possible_gametype_list_unity_vs("848c5980", "000002", &[index.into()])
                .unwrap();
            assert_eq!(matched.len(), 1);
            assert_eq!(matched[0].game_type_name, GPU_TYPE);
            assert!(matched[0].gpu_pre_skinning);
        }
    }

    #[test]
    fn missing_pointlist_log_mapping_keeps_gpu_type() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-gimi-mapping-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("deduped")).unwrap();
        fs::write(root.join("log.txt"), "analyse_options: 0140073d\n").unwrap();
        // 只构造布局和文件链接，不携带游戏数据；三个 Category 都有 316 个顶点。
        for (name, hash, size) in [
            (
                "000002-vb0=54c32104-vs=653c63ba4a73ca8b.buf",
                "54c32104",
                316 * 40,
            ),
            (
                "000002-vb1=61224bdf-vs=653c63ba4a73ca8b.buf",
                "61224bdf",
                316 * 32,
            ),
            ("000073-vb1=9f15c59d-vs=test.buf", "9f15c59d", 316 * 12),
            ("000138-vb1=9f15c59d-vs=test.buf", "9f15c59d", 316 * 12),
        ] {
            let target = root.join("deduped").join(format!("{hash}.buf"));
            fs::write(&target, vec![0u8; size]).unwrap();
            fs::hard_link(target, root.join(name)).unwrap();
        }
        let extractor = fixture_extractor(&root);
        assert!(extractor.fa.log.filename_deduped_filename_map.is_empty());
        assert_gpu_match(&extractor);

        // 即使 Hash 文件仍在，丢失真实链接也不能伪造一次成功匹配。
        fs::remove_file(root.join("000002-vb0=54c32104-vs=653c63ba4a73ca8b.buf")).unwrap();
        let extractor = fixture_extractor(&root);
        assert!(extractor
            .get_possible_gametype_list_unity_vs("848c5980", "000002", &["000073".into()])
            .unwrap()
            .is_empty());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "需要显式提供外部转储目录；只读取，不修改转储或工作空间"]
    fn existing_848c5980_dump_matches_gpu_type() {
        let folder =
            std::env::var_os("SSMT_GIMI_MAPPING_SAMPLE").expect("请设置 SSMT_GIMI_MAPPING_SAMPLE");
        let extractor = fixture_extractor(Path::new(&folder));
        let position = extractor
            .fa
            .log
            .resolve_dump_filepath("000002-vb0=54c32104-vs=653c63ba4a73ca8b.buf")
            .unwrap();
        assert_eq!(fs::metadata(position).unwrap().len(), 316 * 40);
        assert_eq!(
            extractor
                .fa
                .log
                .get_deduped_filename("000002-vb0=54c32104-vs=653c63ba4a73ca8b.buf"),
            "54c32104.buf"
        );
        assert_gpu_match(&extractor);
    }
}
