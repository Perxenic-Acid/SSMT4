use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::common::d3d11_gametype::D3D11GameType;
use crate::common::frame_analysis::frameanalysis_data::FrameAnalysisData;
use crate::constants::gametype::ExtractTechnique;
use crate::gametype;

#[derive(Debug, Default, Clone)]
pub struct D3D11GameTypeLv2 {
    pub d3d11_game_type_list: Vec<D3D11GameType>,
    pub game_type_name_map: HashMap<String, D3D11GameType>,
    pub ordered_gpu_cpu_d3d11_gametype_list: Vec<D3D11GameType>,
}

impl D3D11GameTypeLv2 {
    pub fn unique_game_type_list(&self) -> Vec<D3D11GameType> {
        let mut seen_names: HashSet<String> = HashSet::new();
        let mut unique_list: Vec<D3D11GameType> = Vec::new();

        for gt in &self.ordered_gpu_cpu_d3d11_gametype_list {
            if seen_names.insert(gt.game_type_name.clone()) {
                unique_list.push(gt.clone());
            }
        }

        unique_list
    }

    pub fn new_from_gamename(game_name: &str) -> Result<Self, String> {
        Self::from_game_name(game_name)
    }

    pub fn from_game_name(game_name: &str) -> Result<Self, String> {
        crate::extract_log!("Loading D3D11GameType registry for game: {}", game_name);
        let d3d11_game_type_list = gametype::get_game_type_list(game_name)?;
        Self::from_game_type_list(d3d11_game_type_list)
    }

    /// 插件类型提供者已解析出列表时直接使用，不再走内置注册表。
    pub fn from_type_list(d3d11_game_type_list: Vec<D3D11GameType>) -> Result<Self, String> {
        Self::from_game_type_list(d3d11_game_type_list)
    }

    /// 兼容旧接口：从路径最后一级目录名推断游戏名，再从外部/内部注册表读取。
    pub fn new(game_type_folder: impl AsRef<Path>) -> Result<Self, String> {
        let folder: PathBuf = game_type_folder.as_ref().to_path_buf();
        crate::extract_log!(
            "Loading D3D11GameType from folder alias: {}",
            folder.display()
        );

        let game_name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| format!("Unable to infer game name from path: {}", folder.display()))?;

        Self::from_game_name(game_name)
    }

    fn from_game_type_list(d3d11_game_type_list: Vec<D3D11GameType>) -> Result<Self, String> {
        if d3d11_game_type_list.is_empty() {
            return Err("No D3D11GameType entries found in GameType registry".to_string());
        }

        let mut game_type_name_map: HashMap<String, D3D11GameType> = HashMap::new();
        for gt in &d3d11_game_type_list {
            game_type_name_map.insert(gt.game_type_name.clone(), gt.clone());
        }

        // GPU 优先，再 CPU
        let mut ordered_gpu_cpu_list: Vec<D3D11GameType> = Vec::new();
        for gt in d3d11_game_type_list.iter() {
            if gt.gpu_pre_skinning {
                ordered_gpu_cpu_list.push(gt.clone());
            }
        }
        for gt in d3d11_game_type_list.iter() {
            if !gt.gpu_pre_skinning {
                ordered_gpu_cpu_list.push(gt.clone());
            }
        }

        crate::extract_log!(
            "Loaded {} D3D11GameType entries.",
            d3d11_game_type_list.len()
        );
        Ok(Self {
            d3d11_game_type_list,
            game_type_name_map,
            ordered_gpu_cpu_d3d11_gametype_list: ordered_gpu_cpu_list,
        })
    }

    pub fn filter_trianglelist_index_unity_vs(
        &self,
        frame_analysis_data: &FrameAnalysisData,
        trianglelist_index_list: &[String],
        d3d11_game_type: &D3D11GameType,
    ) -> String {
        for trianglelist_index in trianglelist_index_list.iter() {
            let mut all_slot_exists = true;

            for (category, topology) in d3d11_game_type.category_topology_dict.iter() {
                if topology != ExtractTechnique::TRIANGLELIST {
                    continue;
                }

                let Some(category_slot) = d3d11_game_type.category_slot_dict.get(category) else {
                    all_slot_exists = false;
                    break;
                };

                let search_key = format!("{}-{}", trianglelist_index, category_slot);
                let category_buf_file_name = frame_analysis_data
                    .filter_first_file(&search_key, ".buf")
                    .unwrap_or_default();

                if category_buf_file_name.is_empty() {
                    all_slot_exists = false;
                    break;
                }
            }

            if all_slot_exists {
                return trianglelist_index.clone();
            }
        }

        String::new()
    }
}
