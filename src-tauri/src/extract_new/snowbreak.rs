use std::collections::{BTreeMap, HashMap};
use std::fs;
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
use crate::workspace::submesh_json::{SubMeshCategoryBuffer, SubMeshIndexBuffer, SubMeshJson};

pub struct SnowBreakNewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl SnowBreakNewExtractor {
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
        let current_gametype_folder_path = gametype_folder_path.join("SnowBreak");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract,
            d3d11_gametype_lv2,
        })
    }

    fn get_match_first_index_ibtxt_filename_dict(
        &self,
        draw_ib: &str,
    ) -> Result<BTreeMap<u64, String>, String> {
        let mut out: BTreeMap<u64, String> = BTreeMap::new();
        let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(draw_ib);

        for trianglelist_index in trianglelist_index_list {
            let ib_txt_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                .unwrap_or_default();
            if ib_txt_file_name.is_empty() {
                continue;
            }

            let ib_txt_file_path = self.fa.log.get_deduped_filepath(&ib_txt_file_name);
            if ib_txt_file_path.is_empty() {
                continue;
            }

            let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_file_path, false)?;
            let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
            out.insert(match_first_index, ib_txt_file_name);
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
                let category_slot_file_path =
                    self.fa.log.get_deduped_filepath(&category_slot_file_name);
                if category_slot_file_path.is_empty() {
                    all_slot_match = false;
                    break;
                }

                let category_slot_txt_file_name = trianglelist_index_list
                    .iter()
                    .find_map(|trianglelist_index| {
                        let txt_file_name = self
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
                    .fa
                    .log
                    .get_deduped_filepath(&category_slot_txt_file_name);
                let slot_file_size = if category_slot_txt_file_name.is_empty()
                    || category_slot_txt_file_path.is_empty()
                {
                    SSMTFileUtils::get_file_size(&category_slot_file_path)?
                } else {
                    if d3d11_game_type.gpu_pre_skinning {
                        SSMTFileUtils::get_file_size(&category_slot_file_path)?
                    } else {
                        SSMTBinaryUtils::get_file_size_from_migoto_txt(
                            &category_slot_txt_file_path,
                        )?
                    }
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

    fn export_unreal_vs_submeshes(
        &self,
        game_preset: &str,
        draw_ib: &str,
        max_slot_trianglelist_index: &str,
        trianglelist_index_list: &[String],
        possible_d3d11_game_type_list: &[D3D11GameType],
    ) -> Result<bool, String> {
        if possible_d3d11_game_type_list.is_empty() {
            return Ok(false);
        }

        let match_first_index_ib_txt_file_name_dict =
            self.get_match_first_index_ibtxt_filename_dict(draw_ib)?;
        for (match_first_index, ib_file_name) in &match_first_index_ib_txt_file_name_dict {
            crate::extract_log!(
                "MatchFirstIndex: {} IBFileName: {}",
                match_first_index,
                ib_file_name
            );
        }

        for d3d11_game_type in possible_d3d11_game_type_list {
            let mut category_slot_file_name_dict: HashMap<String, String> = HashMap::new();
            let mut category_slot_txt_file_name_dict: HashMap<String, String> = HashMap::new();
            for trianglelist_index in trianglelist_index_list.iter() {
                for category_slot in d3d11_game_type.category_slot_dict.values() {
                    let category_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", trianglelist_index, category_slot),
                            ".buf",
                        )
                        .unwrap_or_default();
                    let category_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", trianglelist_index, category_slot),
                            ".txt",
                        )
                        .unwrap_or_default();
                    if !category_file_name.is_empty() {
                        category_slot_file_name_dict
                            .insert(category_slot.clone(), category_file_name.clone());
                        category_slot_txt_file_name_dict
                            .insert(category_slot.clone(), category_txt_file_name.clone());
                    }
                }
            }

            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);

            let mut category_buf_file_name_dict: HashMap<String, String> = HashMap::new();
            for (category_name, category_slot) in d3d11_game_type.category_slot_dict.iter() {
                let category_buf_file_name = category_slot_file_name_dict
                    .get(category_slot)
                    .cloned()
                    .unwrap_or_default();
                category_buf_file_name_dict.insert(category_name.clone(), category_buf_file_name);
            }

            let ib_txt_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-ib", max_slot_trianglelist_index), ".txt")
                .unwrap_or_default();
            if ib_txt_file_name.is_empty() {
                crate::extract_log!(
                    "无法找到 Index {} 的IB txt文件，跳过此数据类型",
                    max_slot_trianglelist_index
                );
                continue;
            }

            let ib_txt_file_path = self.fa.log.get_deduped_filepath(&ib_txt_file_name);
            let read_dxgi_format =
                SSMTFileUtils::find_migoto_ini_attribute_in_file(&ib_txt_file_path, "format")?;
            let ib_file_format = if read_dxgi_format == "DXGI_FORMAT_R32_UINT" {
                "DXGI_FORMAT_R32_UINT".to_string()
            } else {
                "DXGI_FORMAT_R16_UINT".to_string()
            };

            let ib_buf_file_name =
                SSMTFileUtils::get_filename_with_new_extension(&ib_txt_file_name, "buf")?;
            let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);

            let vb0_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-vb0", max_slot_trianglelist_index), ".txt")
                .unwrap_or_default();
            let vertex_limit_vb = if vb0_file_name.is_empty() {
                String::new()
            } else {
                vb0_file_name.chars().skip(11).take(8).collect()
            };

            for (_match_first_index, tmp_ib_txt_file_name) in
                match_first_index_ib_txt_file_name_dict.iter()
            {
                let tmp_ib_txt_file_path = self.fa.log.get_deduped_filepath(tmp_ib_txt_file_name);
                if tmp_ib_txt_file_path.is_empty() {
                    continue;
                }

                let tmp_ib_txt_file = IndexBufferTxtFile::new(&tmp_ib_txt_file_path, true)?;
                let index_count = tmp_ib_txt_file.index_number_count as usize;

                let unique_str_folder_name = format!(
                    "{}-{}-{}",
                    draw_ib, tmp_ib_txt_file.index_count, tmp_ib_txt_file.first_index
                );
                let game_type_output_path = PathBuf::from(&self.workspace_path)
                    .join(&unique_str_folder_name)
                    .join(&game_type_folder_name);
                SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;

                let name_prefix = unique_str_folder_name.clone();

                // Export IB (divided per submesh)
                let output_ib_buf_file_path =
                    game_type_output_path.join(format!("{}.ib", name_prefix));
                let mut ib_buf_file =
                    IndexBufferBufFile::from_file(&ib_buf_file_path, &ib_file_format)?;
                ib_buf_file.self_divide(
                    tmp_ib_txt_file.first_index.parse::<usize>().unwrap_or(0),
                    index_count,
                );
                ib_buf_file.save_to_file_uint32(&output_ib_buf_file_path, 0)?;

                // Export raw category buffers per submesh
                for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                    let category_buf_file_name = category_buf_file_name_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_output_buf_file_path = game_type_output_path
                        .join(format!("{}-{}.buf", name_prefix, category_name));
                    self.export_category_buffer(
                        category_name,
                        &category_buf_file_name,
                        d3d11_game_type.gpu_pre_skinning,
                        &category_output_buf_file_path,
                    )?;
                }

                // Build SubMeshJson per submesh
                let mut submesh_json = SubMeshJson::new();
                submesh_json.game_preset = game_preset.to_string();
                submesh_json.vertex_limit_vb = vertex_limit_vb.clone();
                submesh_json.work_game_type = d3d11_game_type.game_type_name.clone();
                submesh_json.gpu_pre_skinning = d3d11_game_type.gpu_pre_skinning;
                submesh_json.index_buffer_list.push(SubMeshIndexBuffer {
                    dxgi_format: "DXGI_FORMAT_R32_UINT".to_string(),
                    file_name: format!("{}.ib", name_prefix),
                });

                for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                    let category_buf_file_name = category_buf_file_name_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_hash = self.build_category_hash_from_buf_file_name(
                        d3d11_game_type,
                        category_name,
                        &category_buf_file_name,
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
                    .save_to_file(game_type_output_path.join(format!("{}.json", name_prefix)))?;
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
            if draw_ib == "8d45cfee" {
                crate::extract_new::log_skipped_drawib(draw_ib, "known fake DrawIB");
                continue;
            } else {
                crate::extract_log!("当前DrawIB: {}", draw_ib);
            }

            let trianglelist_index_list = self.fa.data.get_trianglelist_index_list(&draw_ib);
            if trianglelist_index_list.is_empty() {
                crate::extract_new::log_skipped_drawib(draw_ib, "no trianglelist data files found");
                continue;
            }

            let mut max_slot_number: usize = 0;
            let mut max_slot_trianglelist_index = String::new();

            crate::extract_log!("初始化 CategorySlot Hash Dict:");
            for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("{}", trianglelist_index);
                let category_slot_hash_dict = self
                    .fa
                    .log
                    .get_vb_category_hash_map_from_ia_set_vertex_buffer_by_index(
                        trianglelist_index,
                    );

                if category_slot_hash_dict.len() >= max_slot_number {
                    max_slot_number = category_slot_hash_dict.len();
                    max_slot_trianglelist_index = trianglelist_index.clone();
                }
            }

            crate::extract_log!("TrianglelistIndex: {}", max_slot_trianglelist_index);

            let mut possible_d3d11_game_type_list =
                self.get_possible_gametype_list_unreal_vs(&draw_ib, &trianglelist_index_list)?;
            possible_d3d11_game_type_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));
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

            let extract_success = self.export_unreal_vs_submeshes(
                "SnowBreak",
                draw_ib,
                &max_slot_trianglelist_index,
                &trianglelist_index_list,
                &possible_d3d11_game_type_list,
            )?;

            if !extract_success {
                crate::extract_new::log_skipped_drawib(
                    draw_ib,
                    format!(
                        "no valid data type matched. TrianglelistIndex: {:?}",
                        trianglelist_index_list
                    ),
                );
                continue;
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
