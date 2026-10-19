use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

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

pub struct IdentityVNewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl IdentityVNewExtractor {
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

    fn export_precollected_submeshes(
        &self,
        game_preset: &str,
        draw_ib: &str,
        vertex_limit_vb: &str,
        d3d11_game_type: &D3D11GameType,
        match_first_index_ib_txt_file_name_dict: &BTreeMap<(u64, u64), String>,
    ) -> Result<bool, String> {
        if match_first_index_ib_txt_file_name_dict.is_empty() {
            return Ok(false);
        }

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
                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                let category_buf_filename = self
                    .fa
                    .data
                    .filter_first_file(
                        &format!("{}-{}=", per_ib_trianglelist_index, category_slot),
                        ".buf",
                    )
                    .unwrap_or_default();
                category_buf_filename_map.insert(category_name.clone(), category_buf_filename);
            }
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
                    d3d11_game_type.gpu_pre_skinning,
                    &category_output_buf_file_path,
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

            submesh_json
                .save_to_file(game_type_output_path.join(name_prefix.to_string() + ".json"))?;
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
        let current_gametype_folder_path = gametype_folder_path.join("IdentityV");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            workspace_path: workspace_path.clone(),
            drawib_config,
            specify_drawib_extract,
            d3d11_gametype_lv2,
        })
    }

    pub fn auto_detect_game_type(&self) {}

    fn discover_draw_ib_list_from_frame_analysis(&self) -> Result<Vec<String>, String> {
        let mut discovered_draw_ib_list: Vec<String> = Vec::new();

        for ib_txt_filename in self
            .fa
            .data
            .files
            .iter()
            .filter(|file_name| file_name.contains("-ib=") && file_name.ends_with(".txt"))
        {
            let draw_ib = ib_txt_filename
                .get(10..18)
                .unwrap_or_default()
                .trim()
                .to_string();
            if draw_ib.is_empty() || discovered_draw_ib_list.contains(&draw_ib) {
                continue;
            }

            let ib_txt_filepath = self.fa.log.get_deduped_filepath(ib_txt_filename);
            if ib_txt_filepath.is_empty() || !Path::new(&ib_txt_filepath).exists() {
                continue;
            }

            let ib_txt_file = IndexBufferTxtFile::new(&ib_txt_filepath, false)?;
            if ib_txt_file.topology != "trianglelist" {
                continue;
            }

            discovered_draw_ib_list.push(draw_ib);
        }

        Ok(discovered_draw_ib_list)
    }

    fn get_target_draw_ib_list(&self) -> Result<Vec<String>, String> {
        if !self.specify_drawib_extract {
            return self.discover_draw_ib_list_from_frame_analysis();
        }

        Ok(self
            .drawib_config
            .entries
            .iter()
            .map(|entry| entry.draw_ib.trim().to_string())
            .filter(|draw_ib| !draw_ib.is_empty())
            .collect())
    }

    fn build_sync_drawib_config(&self, extracted_draw_ib_set: &HashSet<String>) -> DrawIBConfig {
        let mut entries = self
            .drawib_config
            .entries
            .iter()
            .filter(|entry| extracted_draw_ib_set.contains(entry.draw_ib.trim()))
            .cloned()
            .collect::<Vec<_>>();

        for draw_ib in extracted_draw_ib_set {
            if entries.iter().any(|entry| entry.draw_ib.trim() == draw_ib) {
                continue;
            }

            entries.push(DrawIBEntry {
                draw_ib: draw_ib.clone(),
                alias: draw_ib.clone(),
            });
        }

        DrawIBConfig {
            path: self.drawib_config.path.clone(),
            entries,
        }
    }

    pub fn extract_model(
        &self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<Vec<String>, String> {
        crate::extract_log!("开始提取:");
        crate::extract_log!("数据类型筛选: {}", data_type_filter.description());
        crate::extract_log!("使用工作空间 DrawIB 列表: {}", self.specify_drawib_extract);

        let mut extracted_draw_ib_list: Vec<String> = Vec::new();
        let target_draw_ib_list = self.get_target_draw_ib_list()?;

        for draw_ib in target_draw_ib_list {
            if draw_ib.is_empty() {
                continue;
            }

            crate::extract_log!("当前DrawIB: {}", draw_ib);

            let trianglelist_ib_file_list = self
                .fa
                .data
                .filter_filelist(&format!("-ib={}", draw_ib), ".txt");

            let mut trianglelist_index_list: Vec<String> = Vec::new();
            crate::extract_log!("TrianglelistIndexList:");
            for trianglelist_ib_file_name in trianglelist_ib_file_list.iter() {
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

            let tmp_ib_buf_file_name = self
                .fa
                .data
                .filter_first_file(&format!("{}-ib=", tmp_trianglelist_index), ".buf")
                .unwrap_or_default();
            if tmp_ib_buf_file_name.is_empty() {
                crate::extract_log!("未找到临时IB Buf文件，跳过当前DrawIB");
                continue;
            }

            let tmp_ib_buf_file_path = self.fa.log.get_deduped_filepath(&tmp_ib_buf_file_name);
            if tmp_ib_buf_file_path.is_empty() {
                crate::extract_log!("临时IB Buf deduped路径为空，跳过当前DrawIB");
                continue;
            }

            let _tmp_ib_buf_file =
                IndexBufferBufFile::from_file(&tmp_ib_buf_file_path, "dxgi_format_r16_uint")?;

            let mut possible_game_type_list: Vec<D3D11GameType> = Vec::new();
            for d3d11_game_type in self
                .d3d11_gametype_lv2
                .ordered_gpu_cpu_d3d11_gametype_list
                .iter()
            {
                crate::extract_log!("尝试匹配数据类型: {}", d3d11_game_type.game_type_name);

                let mut category_name_buf_file_path_dict: HashMap<String, String> = HashMap::new();
                let mut all_slot_buf_file_exists = true;

                for (category_name, category_slot) in d3d11_game_type.category_slot_dict.iter() {
                    let category_buf_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}=", tmp_trianglelist_index, category_slot),
                            ".buf",
                        )
                        .unwrap_or_default();

                    if category_buf_file_name.is_empty() {
                        all_slot_buf_file_exists = false;
                        break;
                    }

                    let category_buf_file_path =
                        self.fa.log.get_deduped_filepath(&category_buf_file_name);
                    if category_buf_file_path.is_empty() {
                        all_slot_buf_file_exists = false;
                        break;
                    }

                    category_name_buf_file_path_dict
                        .insert(category_name.clone(), category_buf_file_path);
                }

                if !all_slot_buf_file_exists {
                    crate::extract_log!(
                        "当前数据类型并非所有的槽位Buffer文件都存在，不满足，跳过。"
                    );
                    continue;
                }

                let mut vertex_number: u64 = 0;
                let mut all_match = true;

                for category_name in d3d11_game_type.ordered_category_name_list.iter() {
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

                    let category_buf_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}=", tmp_trianglelist_index, category_slot),
                            ".txt",
                        )
                        .unwrap_or_default();
                    let category_buf_txt_file_path = self
                        .fa
                        .log
                        .get_deduped_filepath(&category_buf_txt_file_name);
                    if !category_buf_txt_file_name.is_empty()
                        && category_buf_txt_file_path.is_empty()
                    {
                        return Err(format!(
                            "Category {} txt file path is empty: {}",
                            category_name, category_buf_txt_file_name
                        ));
                    }
                    let buf_file_size = if category_buf_txt_file_name.is_empty()
                        || category_buf_txt_file_path.is_empty()
                    {
                        SSMTFileUtils::get_file_size(&buf_file_path)?
                    } else {
                        SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_buf_txt_file_path)?
                    };
                    let tmp_number = buf_file_size / category_stride;

                    let yu_shu = buf_file_size % category_stride;
                    if yu_shu != 0 {
                        crate::extract_log!(
                            "余数不为0: {}，文件步长除以类别步长不能含余数",
                            yu_shu
                        );
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

                    if d3d11_game_type.category_slot_dict.len() == 1 {
                        let category_txt_file_name = self
                            .fa
                            .data
                            .filter_first_file(
                                &format!("{}-{}=", tmp_trianglelist_index, category_slot),
                                ".txt",
                            )
                            .unwrap_or_default();

                        if category_txt_file_name.is_empty() {
                            crate::extract_log!("槽位的txt文件不存在，无法校验顶点数，匹配失败");
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

                        let stride_show_str = SSMTFileUtils::find_migoto_ini_attribute_in_file(
                            &category_txt_file_path,
                            "stride",
                        )?;
                        let stride_show = stride_show_str.parse::<u64>().unwrap_or(0);

                        if stride_show != d3d11_game_type.get_self_stride() {
                            crate::extract_log!("显示步长: {}", stride_show_str);
                            crate::extract_log!(
                                "数据类型步长: {}",
                                d3d11_game_type.get_self_stride()
                            );
                            crate::extract_log!("当前文件中显示步长与数据类型步长不符，匹配失败");
                            all_match = false;
                            break;
                        }
                    }
                }

                if all_match {
                    possible_game_type_list.push(d3d11_game_type.clone());
                }
            }

            if possible_game_type_list.is_empty() {
                crate::extract_log!(
                    "未找到任何匹配的数据类型，跳过当前 DrawIB。DrawIB: {} TrianglelistIndex: {:?}",
                    draw_ib,
                    trianglelist_index_list
                );
                continue;
            }

            crate::extract_log!("当前匹配到的数据类型列表:");
            for d3d11_game_type in possible_game_type_list.iter() {
                crate::extract_log!("{}", d3d11_game_type.game_type_name);
            }

            possible_game_type_list.retain(|d3d11_game_type| {
                data_type_filter.allows(d3d11_game_type.gpu_pre_skinning)
            });
            if possible_game_type_list.is_empty() {
                crate::extract_log!(
                    "当前 DrawIB 在当前数据类型筛选下没有可提取的数据类型，跳过: {}",
                    draw_ib
                );
                continue;
            }

            let all_cpu_game_type = possible_game_type_list
                .iter()
                .all(|d3d11_game_type| !d3d11_game_type.gpu_pre_skinning);

            if all_cpu_game_type {
                let max_stride = possible_game_type_list
                    .iter()
                    .map(|gt| gt.get_self_stride())
                    .max()
                    .unwrap_or(0);

                possible_game_type_list = possible_game_type_list
                    .into_iter()
                    .filter(|gt| gt.get_self_stride() == max_stride)
                    .collect();
            }

            let mut match_first_index_ib_file_name_dict: BTreeMap<(u64, u64), String> =
                BTreeMap::new();
            for trianglelist_index in trianglelist_index_list.iter() {
                let ib_txt_file_name = self
                    .fa
                    .data
                    .filter_first_file(&format!("{}-ib", trianglelist_index), ".txt")
                    .unwrap_or_default();
                if ib_txt_file_name.is_empty() {
                    continue;
                }

                let ib_file_path = self.fa.log.get_deduped_filepath(&ib_txt_file_name);
                if ib_file_path.is_empty() {
                    continue;
                }

                let ib_txt_file = IndexBufferTxtFile::new(&ib_file_path, false)?;
                let match_first_index = ib_txt_file.first_index.parse::<u64>().unwrap_or(0);
                let match_index_count = ib_txt_file.index_count.parse::<u64>().unwrap_or(0);
                // Dedup by the same key used in output folder naming (draw_ib-index_count-first_index).
                // Using only first_index can collapse distinct submeshes into one.
                match_first_index_ib_file_name_dict
                    .insert((match_first_index, match_index_count), ib_txt_file_name);
            }

            for d3d11_game_type in possible_game_type_list.iter() {
                crate::extract_log!("当前提取数据类型: {}", d3d11_game_type.game_type_name);
                let exported = self.export_precollected_submeshes(
                    "IdentityV",
                    &draw_ib,
                    &self.build_vertex_limit_vb(&tmp_trianglelist_index),
                    d3d11_game_type,
                    &match_first_index_ib_file_name_dict,
                )?;

                if exported && !extracted_draw_ib_list.contains(&draw_ib) {
                    extracted_draw_ib_list.push(draw_ib.clone());
                }
            }
        }

        crate::extract_log!("提取正常执行完成");
        if extracted_draw_ib_list.is_empty() {
            return Err("IdentityV 提取失败，未导出任何模型。".to_string());
        }

        Ok(extracted_draw_ib_list)
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        let extracted_draw_ib_list = self.extract_model(data_type_filter)?;
        let extracted_draw_ib_set: HashSet<String> = extracted_draw_ib_list.into_iter().collect();
        let sync_drawib_config = self.build_sync_drawib_config(&extracted_draw_ib_set);
        sync_workspace_deduped_textures_and_json(
            &self.fa,
            &sync_drawib_config,
            &self.workspace_path,
        )?;
        Ok(())
    }
}
