use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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
use std::fs;

pub struct APMI2NewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl APMI2NewExtractor {
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
                    category_buf_filename_map.insert(category_name.clone(), category_buf_file_name);
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
                            d3d11_element_list: self.build_submesh_elements_for_category(
                                d3d11_game_type,
                                category_name,
                            ),
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
            let cs_cb0_filename: String = self.fa.data.filter_first_file(&cs_cb0_key, ".buf")?;
            crate::extract_log!("cs-cb0 filename: {}", cs_cb0_filename);
            crate::extract_log!();

            if cs_cb0_filename.contains("38cddb8c0126726a") {
                let result = self.extract_38cddb8c0126726a(
                    &draw_ib,
                    &pointlist_index,
                    &trianglelist_index_list,
                    data_type_filter,
                )?;

                if !result {
                    crate::extract_new::log_skipped_drawib(
                        draw_ib,
                        format!(
                            "no valid data type matched. PointlistIndex: {} TrianglelistIndex: {:?}",
                            pointlist_index, trianglelist_index_list
                        ),
                    );
                    continue;
                }
            } else {
                let result = self.extract_fee307b98a965c16(
                    &draw_ib,
                    &pointlist_index,
                    &trianglelist_index_list,
                    data_type_filter,
                )?;
                if !result {
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
        }

        crate::extract_log!("提取正常执行完成");
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
