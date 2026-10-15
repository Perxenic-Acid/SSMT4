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
use crate::helper::workspace_texture_sync::sync_cami_workspace_deduped_textures_and_json;
use crate::utils::ssmt_binary_utils::SSMTBinaryUtils;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::workspace::submesh_json::{SubMeshCategoryBuffer, SubMeshIndexBuffer, SubMeshJson};

// 卡拉彼丘 (Calabiyau / Strinova)
//
// 该游戏的蒙皮网格在 trianglelist 绘制时直接绑定骨骼索引/权重缓冲（vb4 8 字节
// = R8G8B8A8_UINT 索引 + R8G8B8A8_UNORM 权重），没有独立于 trianglelist 的
// pointlist 预蒙皮 pass，因此采用 unreal_vs 匹配算法（GPU 类型直接对
// trianglelist 绘制做匹配），与 SnowBreak 的处理方式一致。
//
// 主力布局（角色 mesh，与 SSMT 生成 mod 的资源 stride 对应）：
//   vb0=12(Position) vb1=8(Vector) vb2=4(Texcoord) vb3=4(Color) vb4=8(Blend)
// 部分绘制不绑定 vb3（Color），由无 Color 的 GPU 变体类型覆盖。
pub struct CamiNewExtractor {
    fa: FrameAnalysis,
    workspace_path: String,
    drawib_config: DrawIBConfig,
    specify_drawib_extract: bool,
    d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl CamiNewExtractor {
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

        let gametype_folder_path = PathManager::ssmt_gametype_folder();
        let current_gametype_folder_path = gametype_folder_path.join("CAMI");
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
            // 卡拉彼丘里同一个 IB 还会被 trianglestrip 等非 trianglelist 绘制使用
            // （这类 ib txt 没有 first/count 头），它们不是可提取的子网格，跳过。
            if ib_txt_file.topology != "trianglelist" {
                crate::extract_log!(
                    "跳过非 trianglelist 的 IB txt: {} (topology={})",
                    ib_txt_file_name,
                    ib_txt_file.topology
                );
                continue;
            }
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

        // 卡拉彼丘同一个 IB hash 会被多个不同的子网格绘制复用（各绘制有自己的
        // 顶点缓冲），因此匹配必须按"单次绘制"进行：某个数据类型只要能与其中
        // 任意一个绘制索引的槽位文件完全一致即视为命中。不能像 SnowBreak 那样
        // 把所有 trianglelist 索引的槽位文件混进同一张字典，否则最后一条绘制的
        // 文件会覆盖前面的，造成跨绘制混数据、合法 GPU 类型匹配失败。
        for d3d11_game_type in self
            .d3d11_gametype_lv2
            .ordered_gpu_cpu_d3d11_gametype_list
            .iter()
        {
            if possible_game_type_list
                .iter()
                .any(|matched| matched.gpu_pre_skinning)
                && !d3d11_game_type.gpu_pre_skinning
            {
                crate::extract_log!(
                    "自动优化:已经找到了满足条件的GPU类型，所以这个CPU类型就不用判断了"
                );
                continue;
            }

            crate::extract_log!("当前数据类型: {}", d3d11_game_type.game_type_name);

            let mut matched_for_type = false;

            'per_draw: for trianglelist_index in trianglelist_index_list.iter() {
                crate::extract_log!("TrianglelistIndex: {}", trianglelist_index);

                // 收集当前绘制绑定且属于该数据类型的槽位文件
                let mut category_slot_file_name_dict: HashMap<String, String> = HashMap::new();
                for category_slot in d3d11_game_type.category_slot_dict.values() {
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
                }

                // 该绘制必须绑定数据类型的全部槽位
                if category_slot_file_name_dict.len() != d3d11_game_type.category_slot_dict.len() {
                    crate::extract_log!("当前绘制槽位不完整，尝试下一个绘制索引");
                    continue 'per_draw;
                }

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
                        continue 'per_draw;
                    }

                    let category_slot_file_name = category_slot_file_name_dict
                        .get(category_slot)
                        .cloned()
                        .unwrap_or_default();
                    let category_slot_file_path =
                        self.fa.log.get_deduped_filepath(&category_slot_file_name);
                    if category_slot_file_path.is_empty() {
                        continue 'per_draw;
                    }

                    let category_slot_txt_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", trianglelist_index, category_slot),
                            ".txt",
                        )
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
                            "当前槽位: {} 文件数据不符合当前数据类型要求，尝试下一个绘制索引",
                            category_slot
                        );
                        continue 'per_draw;
                    }
                }

                crate::extract_log!(
                    "识别到数据类型: {} (绘制 {})",
                    d3d11_game_type.game_type_name,
                    trianglelist_index
                );
                matched_for_type = true;
                break 'per_draw;
            }

            if matched_for_type {
                possible_game_type_list.push(d3d11_game_type.clone());
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
        _trianglelist_index_list: &[String],
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
            let game_type_folder_name = format!("TYPE_{}", d3d11_game_type.game_type_name);

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
                // 单个子网格的数据异常只跳过自身，不影响同一 IB 的其他子网格。
                let submesh_result: Result<(), String> = (|| {
                let tmp_ib_txt_file_path = self.fa.log.get_deduped_filepath(tmp_ib_txt_file_name);
                if tmp_ib_txt_file_path.is_empty() {
                    return Ok(());
                }

                // 卡拉彼丘同一个 IB 会被多个不同绘制复用，各绘制有自己的顶点缓冲。
                // 每个子网格必须使用"它所属的那次绘制"的槽位文件：绘制索引即
                // ib txt 文件名的前 6 位。
                let owner_trianglelist_index: String =
                    tmp_ib_txt_file_name.chars().take(6).collect();

                let mut category_buf_file_name_dict: HashMap<String, String> = HashMap::new();
                for category_slot in d3d11_game_type.category_slot_dict.values() {
                    let category_file_name = self
                        .fa
                        .data
                        .filter_first_file(
                            &format!("{}-{}", owner_trianglelist_index, category_slot),
                            ".buf",
                        )
                        .unwrap_or_default();
                    if category_file_name.is_empty() {
                        continue;
                    }
                    category_buf_file_name_dict
                        .insert(category_slot.clone(), category_file_name.clone());
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
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_buf_file_name = category_buf_file_name_dict
                        .get(&category_slot)
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
                    let category_slot = d3d11_game_type
                        .category_slot_dict
                        .get(category_name)
                        .cloned()
                        .unwrap_or_default();
                    let category_buf_file_name = category_buf_file_name_dict
                        .get(&category_slot)
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
                    .save_to_file(game_type_output_path.join(format!("{}.json", name_prefix)))
                    .map_err(|e| format!("Failed to save submesh json for {}: {}", name_prefix, e))?;
                Ok(())
                })();

                if let Err(error) = submesh_result {
                    crate::extract_log!(
                        "跳过子网格 {}: {}",
                        tmp_ib_txt_file_name, error
                    );
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

            let extract_success = match self.export_unreal_vs_submeshes(
                "CAMI",
                draw_ib,
                &max_slot_trianglelist_index,
                &trianglelist_index_list,
                &possible_d3d11_game_type_list,
            ) {
                Ok(success) => success,
                Err(error) => {
                    // 单个 DrawIB 的数据异常（如 deduped 缓冲与 txt 元数据不一致）
                    // 不应中止整个提取流程，记录后继续处理其他 DrawIB。
                    crate::extract_new::log_skipped_drawib(draw_ib, &error);
                    continue;
                }
            };

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
            sync_cami_workspace_deduped_textures_and_json(
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
            sync_cami_workspace_deduped_textures_and_json(
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

    /// 手动验证测试：针对本机真实卡拉彼丘 FA 转储做完整提取 round-trip。
    /// FA 数据不存在时（CI / 其他机器）自动跳过。
    #[test]
    fn round_trip_against_local_calabiyau_fa_dump() {
        let fa_folder = String::from(
            r"C:\Users\Angelwings\Downloads\3Dmigoto_v1.4.10\x64\FrameAnalysis-2026-09-25-234031",
        );
        if !Path::new(&fa_folder).exists() {
            eprintln!("skip: local FA dump not found: {}", fa_folder);
            return;
        }

        let workspace = std::env::temp_dir().join("cami_roundtrip_workspace");
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("Config.json"),
            r#"[{"DrawIB": "f4dca227", "Alias": "character"}]"#,
        )
        .unwrap();

        let mut extractor = CamiNewExtractor::new(
            &fa_folder,
            &workspace.to_string_lossy().to_string(),
            false, // 指定 DrawIB 模式，只提取 Config.json 里列出的 f4dca227
        )
        .expect("extractor init failed");
        extractor
            .run_extract(FullExtractDataTypeFilter::All)
            .expect("run_extract failed");

        // 角色绘制：ib=f4dca227, index_count=72903, first_index=41742
        let out_dir = workspace.join("f4dca227-72903-41742");
        assert!(out_dir.exists(), "output folder missing: {}", out_dir.display());

        let type_dir = std::fs::read_dir(&out_dir)
            .unwrap()
            .flatten()
            .find(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .expect("no TYPE_ folder produced");
        eprintln!("TYPE folder: {}", type_dir.display());

        // 每个 Category 都应导出缓冲，且顶点数一致
        let mut vertex_counts: Vec<u64> = Vec::new();
        for category in ["Position", "Vector", "Texcoord", "Color", "Blend"] {
            let mut found = false;
            for entry in std::fs::read_dir(&type_dir).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(&format!("-{}.buf", category)) {
                    continue;
                }
                found = true;
                let size = entry.metadata().unwrap().len();
                let stride = match category {
                    "Position" => 12,
                    "Vector" | "Blend" => 8,
                    "Texcoord" | "Color" => 4,
                    _ => unreachable!(),
                };
                vertex_counts.push(size / stride);
                break;
            }
            assert!(found, "{} buffer not exported in {}", category, type_dir.display());
        }
        assert!(
            vertex_counts.iter().all(|&v| v == vertex_counts[0] && v > 0),
            "inconsistent vertex counts: {:?}",
            vertex_counts
        );
        eprintln!("round-trip ok, vertex count = {}", vertex_counts[0]);
    }

}
