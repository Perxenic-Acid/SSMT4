use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::workspace::submesh_json::{SubMeshCategoryBuffer, SubMeshIndexBuffer, SubMeshJson};

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
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::utils::ssmt_timer_utils::SSMTTimerUtils;

//EFMI新渲染方式，全局只有一个Buffer。
//在初始化阶段按工作空间 Config.json 中的 DrawIB 列表进行可选过滤。

//1.先通过名称找出所有的DrawIB
//2.对所有的DrawIB进行数据类型识别，只不过要兼容一个Buffer中的不同byte offset的情况
pub struct EFMI3FullNewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    unique_ib_txt_file_list: Vec<IndexBufferTxtFile>,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
    sepcify_drawib_extract: bool,
    specified_draw_ib_hash_set: Option<HashSet<String>>,
    unique_str_index_list_dict: HashMap<String, Vec<String>>,
    unique_str_extracted_dict: HashMap<String, bool>,
}

impl EFMI3FullNewExtractor {
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
        category_output_buf_file_path: &Path,
    ) -> Result<(), String> {
        let category_buf_file_path = self.fa.log.get_deduped_filepath(category_buf_filename);
        if category_buf_file_path.is_empty() {
            return Err(format!(
                "Category {} deduped path is empty: {}",
                category_name, category_buf_filename
            ));
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

    fn normalize_ib_hash(hash: &str) -> String {
        hash.trim().to_ascii_lowercase()
    }

    fn unique_str_matches_specified_draw_ib(&self, unique_str: &str) -> bool {
        if !self.sepcify_drawib_extract {
            return true;
        }

        let Some(set) = &self.specified_draw_ib_hash_set else {
            return true;
        };

        let prefix = unique_str.split('-').next().unwrap_or_default();
        let normalized_prefix = Self::normalize_ib_hash(prefix);
        set.contains(&normalized_prefix)
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
        sepcify_drawib_extract: bool,
    ) -> Result<Self, String> {
        SSMTTimerUtils::start("初始化 EFMI2FullExtractor");
        let frame_analysis_dir = PathBuf::from(frame_analysis_folder);
        if !frame_analysis_dir.exists() {
            return Err(format!(
                "FrameAnalysis 文件夹未找到: {}",
                frame_analysis_folder
            ));
        }

        let fa = FrameAnalysis::new(frame_analysis_folder)?;

        let gametype_folder_path = PathManager::bundled_gametype_folder();
        let current_gametype_folder_path = gametype_folder_path.join("EFMI");
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        let mut unique_mesh_key_set: HashSet<String> = HashSet::new();

        let mut unique_ib_txt_file_list: Vec<IndexBufferTxtFile> = Vec::new();

        //这里我们先要得到所有的DrawIB列表
        //但是其实只有一个DrawIB，只不过每个DrawIB对应的byte offset不同，最终只能通过不同的byte offset来区分不同的Component
        //三个属性byte offset , first index, index count
        //我们找到所有的-ib=的txt文件，根据byte offset将其分为多个Mesh项
        //然后对这些Mesh项进行数据类型识别

        let specified_draw_ib_hash_set: Option<HashSet<String>> =
            DrawIBConfig::read_drawib_list_from_workspace(workspace_path)
                .ok()
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| Self::normalize_ib_hash(&entry.draw_ib))
                        .filter(|draw_ib| !draw_ib.is_empty())
                        .collect::<HashSet<String>>()
                })
                .filter(|set| !set.is_empty());

        let mut unique_str_index_list_dict: HashMap<String, Vec<String>> = HashMap::new();

        for ib_txt_filename in fa
            .data
            .files
            .iter()
            .filter(|f| f.contains("-ib=") && f.ends_with(".txt"))
        {
            let draw_ib = ib_txt_filename.get(10..18).unwrap_or_default().to_string();
            if sepcify_drawib_extract {
                if let Some(draw_ib_set) = &specified_draw_ib_hash_set {
                    let normalized_draw_ib = Self::normalize_ib_hash(&draw_ib);
                    if !draw_ib_set.contains(&normalized_draw_ib) {
                        continue;
                    }
                }
            }

            crate::extract_log!("ib txt filename: {}", ib_txt_filename);

            let ib_txt_filepath = fa.log.get_deduped_filepath(&ib_txt_filename);

            let mut ib_txt_file = IndexBufferTxtFile::new(&ib_txt_filepath, false)?;
            ib_txt_file.fa_filename = ib_txt_filename.clone();

            if ib_txt_file.topology != "trianglelist" {
                continue;
            }

            //由于使用deduped文件初始化会导致文件名异常，所以这里要手动赋值index
            ib_txt_file.index = ib_txt_filename.get(0..6).unwrap_or_default().to_string();
            ib_txt_file.hash = draw_ib;

            // 用于给每个相同的IB文件，分配多个Index值
            let unique_str = format!(
                "{}-{}-{}",
                ib_txt_file.hash.trim(),
                ib_txt_file.index_count.trim(),
                ib_txt_file.first_index.trim(),
            );
            // ib_txt_file.byte_offset.trim(),  暂时不用，但保留用于参考

            // 将当前 ib index 添加到 unique_str_index_list_dict 对应的列表（追加或创建）
            {
                let entry = unique_str_index_list_dict
                    .entry(unique_str.clone())
                    .or_insert_with(Vec::new);
                entry.push(ib_txt_file.index.clone());
            }

            //注意这里插入的是遇到的第一个，但是多个出现的情况下，可能会把具有所有正确槽位的漏掉，所以说还得观察
            //暂时先这样，如果后续提取有bug，再去过滤出槽位数量最多的加入
            if unique_mesh_key_set.insert(unique_str.clone()) {
                unique_ib_txt_file_list.push(ib_txt_file);
            }
        }

        SSMTTimerUtils::end("初始化 EFMI2FullExtractor");

        Ok(Self {
            fa,
            workspace_path: workspace_path.clone(),
            unique_ib_txt_file_list,
            d3d11_gametype_lv2,
            sepcify_drawib_extract,
            specified_draw_ib_hash_set,
            unique_str_index_list_dict,
            unique_str_extracted_dict: HashMap::new(),
        })
    }

    pub fn get_possible_gametype_list_endfield(
        &self,
        ib_txt_file: &IndexBufferTxtFile,
    ) -> Result<Vec<D3D11GameType>, String> {
        let mut possible_gametype_list: Vec<D3D11GameType> = Vec::new();
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

            let trianglelist_index = ib_txt_file.index.clone();

            let mut category_buf_filesize_map: HashMap<String, u64> = HashMap::new();
            let mut all_file_exists = true;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();

                crate::extract_log!(
                    "当前分类: {} 提取Index: {} 提取槽位: {}",
                    category_name,
                    trianglelist_index,
                    category_slot
                );

                let search_key = format!("{}-{}", trianglelist_index, category_slot);
                let category_buf_file_name = self
                    .fa
                    .data
                    .filter_first_file(&search_key, ".buf")
                    .unwrap_or_default();

                crate::extract_log!("CategoryBuf Buffer FileName: {}", category_buf_file_name);

                let category_buf_txt_file_name = self
                    .fa
                    .data
                    .filter_first_file(&search_key, ".txt")
                    .unwrap_or_default();

                crate::extract_log!("CategoryBuf Txt FileName: {}", category_buf_txt_file_name);

                let category_buf_file_path =
                    self.fa.log.get_deduped_filepath(&category_buf_file_name);
                let category_buf_txt_file_path = self
                    .fa
                    .log
                    .get_deduped_filepath(&category_buf_txt_file_name);

                crate::extract_log!(
                    "Category: {} File: {}",
                    category_name,
                    category_buf_file_path
                );

                if category_buf_file_path.is_empty() || !Path::new(&category_buf_file_path).exists()
                {
                    crate::extract_log!("对应Buffer文件未找到,此数据类型无效。");
                    all_file_exists = false;
                    break;
                }

                //这里不是简单的直接获取buf的文件大小
                //而是先读取txt文件获取其byte_offset,stride,vertex count属性，来得到文件大小
                //好像stride + vertex count就能得到了，哦不，好像直接stride就能得到了
                //但是我们保持传统

                let file_size = if category_buf_txt_file_name.is_empty()
                    || category_buf_txt_file_path.is_empty()
                {
                    SSMTFileUtils::get_file_size(&category_buf_file_path)?
                } else {
                    SSMTBinaryUtils::get_file_size_from_migoto_txt(&category_buf_txt_file_path)?
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
                    //crate::extract_log!("槽位的文件大小不能为0，槽位匹配失败，跳过此数据类型");
                    all_match = false;
                    break;
                }

                if !d3d11_game_type.gpu_pre_skinning {
                    let yu_shu = file_size % category_stride;
                    if yu_shu != 0 {
                        //crate::extract_log!("余数不为0: {}，槽位匹配失败", yu_shu);
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
                                //crate::extract_log!("槽位的txt文件Stride与数据类型Stride不符，跳过此数据类型。");
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

        if !possible_gametype_list.is_empty() {
            crate::extract_log!("All Matched GameType:");
            for gt in possible_gametype_list.iter() {
                crate::extract_log!("{}", gt.game_type_name);
            }
        }

        Ok(possible_gametype_list)
    }

    //提取当前ib_txt_file的模型
    pub fn extract_model(
        &self,
        ib_txt_file: &IndexBufferTxtFile,
        possible_d3d11_game_type_list: &Vec<D3D11GameType>,
        suffix_str: &String,
    ) -> Result<bool, String> {
        //确定当前DrawIB中，当前ib txt文件对应输出目录
        let drawib_folder_name = format!("{}-{}", ib_txt_file.hash, suffix_str);
        let draw_ib_folder_path =
            PathBuf::from(&self.workspace_path).join(drawib_folder_name.clone());
        SSMTFileUtils::create_folder_if_not_exists(&draw_ib_folder_path)?;

        //同样输出每一种可能的数据类型，防止遗漏
        for d3d11_game_type in possible_d3d11_game_type_list.iter() {
            let trianglelist_index = ib_txt_file.index.clone();

            //当前数据类型的输出文件夹
            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);
            let game_type_output_path = draw_ib_folder_path.join(game_type_folder_name);
            SSMTFileUtils::create_folder_if_not_exists(&game_type_output_path)?;

            //读取每个CategoryBuffer的真实数据
            //注意这里要先读取到byteoffset, stride和vertex count，来从对应buffer文件中截取数据
            let mut category_buf_file_name_dict: HashMap<String, String> = HashMap::new();
            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let category_slot = d3d11_game_type
                    .category_slot_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();

                let search_key = format!("{}-{}", trianglelist_index, category_slot);
                //txt的文件名和路径，用于读取byte offset和vertex count
                let category_buf_txt_file_name =
                    self.fa.data.filter_first_file(&search_key, ".txt")?;
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

                //buffer的文件名和路径
                let category_buf_file_name = self.fa.data.filter_first_file(&search_key, ".buf")?;
                category_buf_file_name_dict
                    .insert(category_name.clone(), category_buf_file_name.clone());
            }

            let ib_txt_filename = ib_txt_file.fa_filename.clone();
            let ib_buf_file_name =
                SSMTFileUtils::get_filename_with_new_extension(&ib_txt_filename, "buf")?;

            let ib_buf_file_path = self.fa.log.get_deduped_filepath(&ib_buf_file_name);

            let name_prefix = format!("{}-{}", ib_txt_file.hash, suffix_str);
            let output_ib_buf_file_path = game_type_output_path.join(format!("{}.ib", name_prefix));

            let byte_offset_int = ib_txt_file.byte_offset.trim().parse::<usize>().unwrap_or(0);

            let index_element_size = match ib_txt_file.format.to_ascii_lowercase().as_str() {
                "dxgi_format_r16_uint" => 2usize,
                "dxgi_format_r32_uint" => 4usize,
                other => {
                    return Err(format!(
                        "Unsupported IB format in EFMI2 extractor: {}",
                        other
                    ));
                }
            };
            let index_count_int = ib_txt_file.index_count.trim().parse::<usize>().unwrap_or(0);
            let read_length = index_count_int
                .checked_mul(index_element_size)
                .ok_or_else(|| {
                    format!(
                        "read_length overflow: index_count={} element_size={}",
                        index_count_int, index_element_size
                    )
                })?;

            let ib_buf_file = IndexBufferBufFile::from_file_byteoffset_read_length(
                &ib_buf_file_path,
                &ib_txt_file.format,
                byte_offset_int,
                read_length,
            )?;

            // Already sliced by byte_offset/read_length, no second self_divide is needed.
            ib_buf_file
                .save_to_file_uint32(&output_ib_buf_file_path, -(ib_buf_file.min_number as i32))?;

            for category_name in d3d11_game_type.ordered_category_name_list.iter() {
                let category_buf_file_name = category_buf_file_name_dict
                    .get(category_name)
                    .cloned()
                    .unwrap_or_default();
                let category_output_buf_file_path =
                    game_type_output_path.join(format!("{}-{}.buf", name_prefix, category_name));
                self.export_category_buffer(
                    category_name,
                    &category_buf_file_name,
                    &category_output_buf_file_path,
                )?;
            }

            let mut submesh_json = SubMeshJson::new();
            submesh_json.game_preset = "EFMI".to_string();
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
                        d3d11_element_list: self
                            .build_submesh_elements_for_category(d3d11_game_type, category_name),
                    });
            }

            submesh_json
                .save_to_file(game_type_output_path.join(format!("{}.json", name_prefix)))?;
        }

        Ok(true)
    }

    pub fn run_extract(
        &mut self,
        data_type_filter: FullExtractDataTypeFilter,
    ) -> Result<(), String> {
        let mut _suffix_count = 0;

        SSMTTimerUtils::start("提取ib txt文件中的模型");

        for ib_txt_file in self.unique_ib_txt_file_list.iter() {
            crate::extract_log!("Unique IB Txt File: {}", ib_txt_file.file_name);
            crate::extract_log!(
                "First Index: {} Index Count: {} Byte Offset: {}",
                ib_txt_file.first_index,
                ib_txt_file.index_count,
                ib_txt_file.byte_offset
            );
            SSMTTimerUtils::start("数据类型识别测试");
            let unique_str = format!(
                "{}-{}-{}",
                ib_txt_file.hash.trim(),
                ib_txt_file.index_count.trim(),
                ib_txt_file.first_index.trim(),
            );

            //对当前ib_txt_file识别数据类型
            let mut matched_d3d11gametype_list =
                self.get_possible_gametype_list_endfield(&ib_txt_file)?;

            matched_d3d11gametype_list.retain(|gt| data_type_filter.allows(gt.gpu_pre_skinning));

            if !matched_d3d11gametype_list.is_empty() {
                let suffix_str = format!("{}-{}", ib_txt_file.index_count, ib_txt_file.first_index);

                let _result =
                    self.extract_model(ib_txt_file, &matched_d3d11gametype_list, &suffix_str);

                _suffix_count += 1;

                self.unique_str_extracted_dict
                    .insert(unique_str.clone(), true);
            }

            SSMTTimerUtils::end("数据类型识别测试");
        }

        SSMTTimerUtils::end("提取ib txt文件中的模型");

        // for (unique_str, index_list) in self.unique_str_index_list_dict.iter() {
        // 	crate::extract_log!("Unique Str: {} Index List: {:?}", unique_str, index_list);
        // }
        // crate::extract_log!("Total Suffix Count: {}", suffix_count);

        // 根据 unique_str_index_list_dict 中的每个 index，生成工作空间下的总 DedupedTextures 文件夹
        {
            let deduped_folder_path = PathBuf::from(&self.workspace_path).join("DedupedTextures");
            let _ = SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path);

            SSMTTimerUtils::start("复制DedupedTextures到工作空间");
            for (unique_str, index_list) in self.unique_str_index_list_dict.iter() {
                if !self.unique_str_matches_specified_draw_ib(unique_str) {
                    continue;
                }

                let exteacted = self
                    .unique_str_extracted_dict
                    .get(unique_str)
                    .copied()
                    .unwrap_or(false);
                if !exteacted {
                    //crate::extract_log!("Unique Str: {} has not been extracted, skipping copying deduped textures.", unique_str);
                    continue;
                }
                //crate::extract_log!("Unique Str: {} Index List: {:?}", unique_str, index_list);
                for trianglelist_index in index_list.iter() {
                    let content_str = format!("{}-ps-t", trianglelist_index);
                    let ps_texture_all_filename_list =
                        self.fa.data.filter_texture_filename_list(&content_str);

                    for ps_texture_filename in ps_texture_all_filename_list {
                        let deduped_filepath =
                            self.fa.log.get_deduped_filepath(&ps_texture_filename);
                        if deduped_filepath.is_empty() {
                            crate::extract_log!(
                                "No deduped file found for texture: {}",
                                ps_texture_filename
                            );
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
            SSMTTimerUtils::end("复制DedupedTextures到工作空间");
        }

        //根据unique_str_extracted_dict，移除掉unique_str_index_list_dict中对应的为false的项
        self.unique_str_index_list_dict.retain(|unique_str, _| {
            self.unique_str_extracted_dict
                .get(unique_str)
                .copied()
                .unwrap_or(false)
        });

        // 生成工作空间级别的 TextureMark JSON（Index 来源为 unique_str_index_list_dict）
        {
            // 1) ComponentName_DrawCallIndexList.json
            let component_json_path =
                get_workspace_component_name_draw_call_index_list_json_path(&self.workspace_path)?;
            let component_map = self.unique_str_index_list_dict.clone();
            let component_json = ComponentNameDrawCallIndexListJson::from_map(component_map);
            if let Err(e) = component_json.save_to_file(&component_json_path) {
                crate::extract_error!(
                    "Failed to write {}: {}",
                    component_json_path.to_string_lossy(),
                    e
                );
            }

            // 2) TrianglelistDedupedFileName.json
            let mut trianglelist_texture_file_name_list: Vec<String> = Vec::new();
            for (unique_str, index_list) in self.unique_str_index_list_dict.iter() {
                if !self.unique_str_matches_specified_draw_ib(unique_str) {
                    continue;
                }

                for trianglelist_index in index_list.iter() {
                    let content_str = format!("{}-ps-t", trianglelist_index);
                    let ps_texture_all_filename_list =
                        self.fa.data.filter_texture_filename_list(&content_str);
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
                        .fa
                        .log
                        .get_deduped_filename(&trianglelist_texture_file_name);
                    if deduped.trim().is_empty() {
                        String::new()
                    } else {
                        format!("{}_{}", hash, deduped)
                    }
                };

                let fa_data_deduped_filename = {
                    let mut out = String::new();
                    let deduped_dir = Path::new(&self.fa.folder_path).join("deduped");
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
            let trianglelist_json =
                TrianglelistDedupedFileNameJson::from_map(trianglelist_deduped_map);
            if let Err(e) = trianglelist_json.save_to_file(&trianglelist_json_path) {
                crate::extract_error!(
                    "Failed to write {}: {}",
                    trianglelist_json_path.to_string_lossy(),
                    e
                );
            }
        }

        SSMTTimerUtils::show_all();
        Ok(())
    }
}
