use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::config::drawib_config::DrawIBConfig;
use crate::helper::mark_texture_helper::MarkTextureHelper;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use crate::utils::ssmt_string_utils::SSMTStringUtils;
use std::path::{Path, PathBuf};

use crate::extract_new::ailimit::AILIMITNewExtractor;
use crate::extract_new::apmi2::APMI2NewExtractor;
use crate::extract_new::cami::CamiNewExtractor;
use crate::extract_new::efmi3::EFMI3FullNewExtractor;
use crate::extract_new::gf2::GF2NewExtractor;
use crate::extract_new::gimi::GIMINewExtractor;
use crate::extract_new::himi::HIMINewExtractor;
use crate::extract_new::identityv::IdentityVNewExtractor;
use crate::extract_new::naraka::NarakaNewExtractor;
use crate::extract_new::narakam::NarakaMNewExtractor;
use crate::extract_new::ntemi::NTEMINewExtractor;
use crate::extract_new::snowbreak::SnowBreakNewExtractor;
use crate::extract_new::srmi::SRMINewExtractor;
use crate::extract_new::wwmi::WWMINewExtractor;
use crate::extract_new::yysls::YYSLSNewExtractor;
use crate::extract_new::zzmi::ZZMINewExtractor;
use crate::extract_new::zzmi_dx12::ZZMIDX12NewExtractor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullExtractDataTypeFilter {
    All,
    GpuPreSkinningOnly,
    CpuPreSkinningOnly,
}

impl FullExtractDataTypeFilter {
    pub fn from_input(value: &str) -> Result<Self, String> {
        match value.trim() {
            "all" => Ok(Self::All),
            "gpu-preskinning-only" => Ok(Self::GpuPreSkinningOnly),
            "cpu-preskinning-only" => Ok(Self::CpuPreSkinningOnly),
            other => Err(format!(
                "Unsupported full extract data type filter: {}",
                other
            )),
        }
    }

    pub fn allows(self, is_gpu_preskinning: bool) -> bool {
        match self {
            Self::All => true,
            Self::GpuPreSkinningOnly => is_gpu_preskinning,
            Self::CpuPreSkinningOnly => !is_gpu_preskinning,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::All => "所有数据类型",
            Self::GpuPreSkinningOnly => "仅 GPU-PreSkinning 数据类型",
            Self::CpuPreSkinningOnly => "仅 CPU-PreSkinning 数据类型",
        }
    }
}

pub struct ExtractNewService;

impl ExtractNewService {
    pub fn run_extract_dx12(
        frame_analysis_folder: &str,
        lod_workspace_path: &str,
        data_type_filter: FullExtractDataTypeFilter,
        is_full_extract: bool,
    ) -> Result<(), String> {
        crate::extract_log!("[ZZMIDX12][Service] Start DX12 extract flow");
        crate::extract_log!(
            "[ZZMIDX12][Service] FrameAnalysis folder: {}",
            frame_analysis_folder
        );
        crate::extract_log!(
            "[ZZMIDX12][Service] LOD workspace path: {}",
            lod_workspace_path
        );
        crate::extract_log!("[ZZMIDX12][Service] Full extract: {}", is_full_extract);
        if is_full_extract {
            crate::extract_log!(
                "[ZZMIDX12][Service] Data type filter: {}",
                data_type_filter.description()
            );
        }

        let effective_filter = if is_full_extract {
            data_type_filter
        } else {
            FullExtractDataTypeFilter::All
        };
        let mut extractor = ZZMIDX12NewExtractor::new_dx12(
            &frame_analysis_folder.to_string(),
            &lod_workspace_path.to_string(),
            is_full_extract,
        )?;
        let result = extractor.run_extract(effective_filter);

        if result.is_ok() {
            MarkTextureHelper::generate_draw_ib_component_json(lod_workspace_path);
        }

        result
    }

    pub fn extract_deduped_textures(
        fa: &FrameAnalysis,
        game_preset: &str,
        _workspace_root_path: &str,
        lod_workspace_path: &str,
    ) -> Result<(), String> {
        crate::extract_log!("开始提取去重纹理:");
        crate::extract_log!("FrameAnalysis 文件夹路径: {}", fa.folder_path);
        crate::extract_log!("当前游戏预设: {}", game_preset);
        crate::extract_log!("当前LOD工作空间路径: {}", lod_workspace_path);

        let drawib_entries = DrawIBConfig::read_drawib_list_from_workspace(lod_workspace_path)
            .map_err(|e| format!("Failed to read DrawIB list: {}", e))?;

        for drawib_entry in drawib_entries {
            crate::extract_log!(
                "DrawIB: {}, Alias: {}",
                drawib_entry.draw_ib,
                drawib_entry.alias
            );
            let draw_ib = drawib_entry.draw_ib;

            let draw_ib_output_folder_path = PathBuf::from(lod_workspace_path).join(&draw_ib);
            let _ = SSMTFileUtils::create_folder_if_not_exists(&draw_ib_output_folder_path);

            let deduped_folder_path = draw_ib_output_folder_path.join("DedupedTextures");
            let _ = SSMTFileUtils::create_folder_if_not_exists(&deduped_folder_path);

            let trianglelist_index_list = fa.data.get_trianglelist_index_list(&draw_ib);

            let mut trianglelist_texture_file_list: Vec<String> = Vec::new();
            for trianglelist_index in trianglelist_index_list {
                let content_str = trianglelist_index + "-ps-t";
                let ps_texture_all_filename_list =
                    fa.data.filter_texture_filename_list(&content_str);
                trianglelist_texture_file_list.extend(ps_texture_all_filename_list);
            }

            for ps_texture_filename in trianglelist_texture_file_list {
                let deduped_filepath = fa.log.get_deduped_filepath(&ps_texture_filename);
                if deduped_filepath.is_empty() {
                    crate::extract_log!(
                        "No deduped file found for texture: {}",
                        ps_texture_filename
                    );
                    continue;
                }

                let deduped_filename = fa.log.get_deduped_filename(&ps_texture_filename);
                let texture_unique_hash =
                    SSMTStringUtils::get_file_hash_from_file_name(&ps_texture_filename);

                let target_texture_path = deduped_folder_path
                    .join(format!("{}_{}", texture_unique_hash, deduped_filename));

                let target_texture_path_str = target_texture_path.to_string_lossy().to_string();
                let _ = SSMTFileUtils::copy_to_file_if_not_exists(
                    &deduped_filepath,
                    &target_texture_path_str,
                );
            }
        }

        Ok(())
    }

    pub fn extract_trianglelist_textures(
        fa: &FrameAnalysis,
        game_preset: &str,
        _workspace_root_path: &str,
        lod_workspace_path: &str,
    ) -> Result<(), String> {
        crate::extract_log!("开始提取 TrianglelistTextures:");
        crate::extract_log!("当前游戏预设: {}", game_preset);
        crate::extract_log!("当前LOD工作空间路径: {}", lod_workspace_path);

        let drawib_entries = DrawIBConfig::read_drawib_list_from_workspace(lod_workspace_path)
            .map_err(|e| format!("Failed to read DrawIB list: {}", e))?;

        for drawib_entry in drawib_entries {
            crate::extract_log!(
                "DrawIB: {}, Alias: {}",
                drawib_entry.draw_ib,
                drawib_entry.alias
            );
            let draw_ib = drawib_entry.draw_ib;

            let draw_ib_output_folder_path = PathBuf::from(lod_workspace_path).join(&draw_ib);
            let _ = SSMTFileUtils::create_folder_if_not_exists(&draw_ib_output_folder_path);

            let trianglelist_textures_folder_path =
                draw_ib_output_folder_path.join("TrianglelistTextures");
            let _ = SSMTFileUtils::create_folder_if_not_exists(&trianglelist_textures_folder_path);

            let trianglelist_index_list = fa.data.get_trianglelist_index_list(&draw_ib);
            crate::extract_log!(
                "Found {} drawcalls for DrawIB {}",
                trianglelist_index_list.len(),
                draw_ib
            );

            let mut trianglelist_texture_file_list: Vec<String> = Vec::new();
            for trianglelist_index in trianglelist_index_list {
                let content_str = trianglelist_index + "-ps-t";
                let ps_texture_all_filename_list =
                    fa.data.filter_texture_filename_list(&content_str);
                trianglelist_texture_file_list.extend(ps_texture_all_filename_list);
            }

            crate::extract_log!(
                "Found {} textures for DrawIB {}",
                trianglelist_texture_file_list.len(),
                draw_ib
            );
            for ps_texture_filename in trianglelist_texture_file_list {
                let original_ps_texture_filepath =
                    Path::new(&fa.folder_path).join(&ps_texture_filename);
                let target_ps_texture_filepath =
                    trianglelist_textures_folder_path.join(&ps_texture_filename);

                if original_ps_texture_filepath.exists() {
                    let _ = SSMTFileUtils::copy_to_file_if_not_exists(
                        original_ps_texture_filepath.to_str().unwrap_or(""),
                        target_ps_texture_filepath.to_str().unwrap_or(""),
                    );
                }
            }
        }

        Ok(())
    }

    pub fn run_extract(
        fa: &FrameAnalysis,
        game_preset: &str,
        _workspace_root_path: &str,
        lod_workspace_path: &str,
        data_type_filter: FullExtractDataTypeFilter,
        is_full_extract: bool,
    ) -> Result<(), String> {
        crate::extract_log!("开始新提取流程:");
        crate::extract_log!("FrameAnalysis 文件夹路径: {}", fa.folder_path);
        crate::extract_log!("当前游戏预设: {}", game_preset);
        crate::extract_log!("当前LOD工作空间路径: {}", lod_workspace_path);
        crate::extract_log!("是否全量提取: {}", is_full_extract);
        if is_full_extract {
            crate::extract_log!("数据类型筛选: {}", data_type_filter.description());
        }

        let workspace_path_string = lod_workspace_path.to_string();
        let effective_filter = if is_full_extract {
            data_type_filter
        } else {
            FullExtractDataTypeFilter::All
        };
        let result: Result<(), String> = match game_preset {
            "GIMI" => {
                let mut extractor = GIMINewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "HIMI" => {
                let mut extractor = HIMINewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "SRMI" => {
                let mut extractor = SRMINewExtractor::new(
                    fa.folder_path.to_string(),
                    workspace_path_string.clone(),
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "ZZMI" => {
                let mut extractor = ZZMINewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "ZZMIDX12" => {
                let mut extractor = ZZMIDX12NewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "WWMI" => {
                let mut extractor = WWMINewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "EFMI" => {
                let mut extractor = EFMI3FullNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "GF2" => {
                let mut extractor =
                    GF2NewExtractor::new(&fa.folder_path, &workspace_path_string, is_full_extract)?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "AILIMIT" => {
                let mut extractor = AILIMITNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "IdentityV" => {
                let mut extractor = IdentityVNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "NTEMI" => {
                let mut extractor = NTEMINewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "SnowBreak" => {
                let mut extractor = SnowBreakNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "CAMI" => {
                let mut extractor = CamiNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "APMI" => {
                let mut extractor = APMI2NewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "Naraka" => {
                let mut extractor = NarakaNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "NarakaM" => {
                let mut extractor = NarakaMNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            "YYSLS" => {
                let mut extractor = YYSLSNewExtractor::new(
                    &fa.folder_path,
                    &workspace_path_string,
                    is_full_extract,
                )?;
                extractor.run_extract(effective_filter)?;
                Ok(())
            }
            _ => {
                if is_full_extract {
                    Err(format!("SSMT4仍处于Alpha测试阶段，当前游戏预设 {} 暂未添加提取支持，如特别需要请联系NicoMico优先为此游戏添加支持", game_preset))
                } else {
                    Err(format!("新提取模型暂未支持当前游戏预设: {}", game_preset))
                }
            }
        };

        if result.is_ok() {
            MarkTextureHelper::generate_draw_ib_component_json(lod_workspace_path);
        }

        result
    }
}
