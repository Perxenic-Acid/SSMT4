use std::path::PathBuf;

use crate::common::d3d11_gametype_lv2::D3D11GameTypeLv2;
use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::config::drawib_config::DrawIBConfig;
use crate::config::path_manager::PathManager;

pub struct FrameAnalysisBase {
    pub fa: FrameAnalysis,
    pub workspace_path: String,
    pub drawib_config: DrawIBConfig,
    pub specify_drawib_extract: bool,
    pub draw_ib_list: Vec<String>,
    pub d3d11_gametype_lv2: D3D11GameTypeLv2,
}

impl FrameAnalysisBase {
    pub fn new(
        frame_analysis_folder: &str,
        workspace_path: &str,
        specify_drawib_extract: bool,
        game_type_folder_name: &str,
    ) -> Result<Self, String> {
        let frame_analysis_dir = PathBuf::from(frame_analysis_folder);
        if !frame_analysis_dir.exists() {
            return Err(format!(
                "FrameAnalysis 文件夹未找到: {}",
                frame_analysis_folder
            ));
        }

        let drawib_config = if specify_drawib_extract {
            DrawIBConfig::new_from_workspace(workspace_path)
                .map_err(|e| format!("Failed to read DrawIB config: {}", e))?
        } else {
            DrawIBConfig {
                path: String::new(),
                entries: Vec::new(),
            }
        };

        let fa = FrameAnalysis::new(frame_analysis_folder)?;
        let draw_ib_list = if specify_drawib_extract {
            drawib_config
                .entries
                .iter()
                .map(|entry| entry.draw_ib.trim().to_string())
                .filter(|draw_ib| !draw_ib.is_empty())
                .collect::<Vec<String>>()
        } else {
            fa.data.get_all_drawib_list()
        };

        let current_gametype_folder_path =
            PathManager::bundled_gametype_folder().join(game_type_folder_name);
        let d3d11_gametype_lv2 = D3D11GameTypeLv2::new(current_gametype_folder_path)?;

        Ok(Self {
            fa,
            workspace_path: workspace_path.to_string(),
            drawib_config,
            specify_drawib_extract,
            draw_ib_list,
            d3d11_gametype_lv2,
        })
    }
}
