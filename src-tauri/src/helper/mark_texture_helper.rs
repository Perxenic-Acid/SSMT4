use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::common::frame_analysis::frameanalysis::FrameAnalysis;
use crate::config::drawib_config::DrawIBConfig;
use crate::utils::ssmt_string_utils::SSMTStringUtils;
use serde::{Deserialize, Serialize};

pub fn get_workspace_component_name_draw_call_index_list_json_path(
    workspace_path: &str,
) -> Result<PathBuf, String> {
    Ok(PathBuf::from(workspace_path).join("ComponentName_DrawCallIndexList.json"))
}

pub fn get_workspace_trianglelist_deduped_filename_json_path(
    workspace_path: &str,
) -> Result<PathBuf, String> {
    Ok(PathBuf::from(workspace_path).join("TrianglelistDedupedFileName.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComponentNameDrawCallIndexListJson {
    pub component_draw_call_index_list: HashMap<String, Vec<String>>,
}

impl ComponentNameDrawCallIndexListJson {
    pub fn from_map(component_draw_call_index_list: HashMap<String, Vec<String>>) -> Self {
        Self {
            component_draw_call_index_list,
        }
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let content = serde_json::to_string_pretty(&self.component_draw_call_index_list)
            .map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let component_draw_call_index_list: HashMap<String, Vec<String>> =
            serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(Self::from_map(component_draw_call_index_list))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrianglelistDedupedTextureProperty {
    #[serde(rename = "FALogDedupedFileName")]
    pub fa_log_deduped_file_name: String,
    #[serde(rename = "FADataDedupedFileName")]
    pub fa_data_deduped_file_name: String,
    #[serde(default, rename = "Format")]
    pub format: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrianglelistDedupedFileNameJson {
    pub texture_property_map: HashMap<String, TrianglelistDedupedTextureProperty>,
}

impl TrianglelistDedupedFileNameJson {
    pub fn from_map(
        texture_property_map: HashMap<String, TrianglelistDedupedTextureProperty>,
    ) -> Self {
        Self {
            texture_property_map,
        }
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let content =
            serde_json::to_string_pretty(&self.texture_property_map).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let texture_property_map: HashMap<String, TrianglelistDedupedTextureProperty> =
            serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(Self::from_map(texture_property_map))
    }
}

pub struct MarkTextureHelper;

impl MarkTextureHelper {
    /// Generate `ComponentName_DrawCallIndexList.json` for each extracted DrawIB folder.
    ///
    /// This method intentionally does not return a value. Errors are logged and skipped.
    pub fn generate_component_name_draw_call_index_list_json(
        fa: &FrameAnalysis,
        workspace_path: &str,
    ) {
        let drawib_entries = match DrawIBConfig::read_drawib_list_from_workspace(workspace_path) {
            Ok(entries) => entries,
            Err(e) => {
                crate::extract_error!("Failed to read DrawIB list from workspace: {}", e);
                return;
            }
        };

        for drawib_entry in drawib_entries {
            let draw_ib = drawib_entry.draw_ib;
            let drawib_folder = Path::new(workspace_path).join(&draw_ib);

            // Skip DrawIB folders that were not extracted.
            if !drawib_folder.exists() {
                continue;
            }

            let component_map = match fa.data.read_component_name_match_first_index_dict(&draw_ib) {
                Ok(map) => map,
                Err(e) => {
                    crate::extract_error!(
                        "Failed to read component match-first-index map for DrawIB {}: {}",
                        draw_ib,
                        e
                    );
                    continue;
                }
            };

            let mut component_drawcall_map: HashMap<String, Vec<String>> = HashMap::new();

            for component_name in component_map.keys() {
                match fa.data.read_draw_call_index_list(&draw_ib, component_name) {
                    Ok(draw_call_index_list) => {
                        component_drawcall_map.insert(component_name.clone(), draw_call_index_list);
                    }
                    Err(e) => {
                        crate::extract_error!(
                            "Failed to read draw call index list for DrawIB {} component {}: {}",
                            draw_ib,
                            component_name,
                            e
                        );
                    }
                }
            }

            let save_json_file_path = drawib_folder.join("ComponentName_DrawCallIndexList.json");
            let json_model = ComponentNameDrawCallIndexListJson::from_map(component_drawcall_map);
            if let Err(e) = json_model.save_to_file(&save_json_file_path) {
                crate::extract_error!(
                    "Failed to write {}: {}",
                    save_json_file_path.to_string_lossy(),
                    e
                );
            }
        }
    }

    /// Generate `TrianglelistDedupedFileName.json` for each extracted DrawIB folder.
    ///
    /// JSON shape:
    /// {
    ///   "<TrianglelistTextureFileName>": {
    ///     "FALogDedupedFileName": "<hash>_<deduped_from_log>",
    ///     "FADataDedupedFileName": "<hash>_<deduped_from_data_folder_or_empty>"
    ///   }
    /// }
    pub fn generate_trianglelist_deduped_filename_json(fa: &FrameAnalysis, workspace_path: &str) {
        let drawib_entries = match DrawIBConfig::read_drawib_list_from_workspace(workspace_path) {
            Ok(entries) => entries,
            Err(e) => {
                crate::extract_error!("Failed to read DrawIB list from workspace: {}", e);
                return;
            }
        };

        for drawib_entry in drawib_entries {
            let draw_ib = drawib_entry.draw_ib;
            let drawib_folder = Path::new(workspace_path).join(&draw_ib);
            if !drawib_folder.exists() {
                continue;
            }

            let trianglelist_index_list = fa.data.get_trianglelist_index_list(&draw_ib);
            let mut trianglelist_texture_file_name_list: Vec<String> = Vec::new();

            for trianglelist_index in trianglelist_index_list {
                let content_str = trianglelist_index + "-ps-t";
                let ps_texture_all_filename_list =
                    fa.data.filter_texture_filename_list(&content_str);
                trianglelist_texture_file_name_list.extend(ps_texture_all_filename_list);
            }

            let mut trianglelist_deduped_map: HashMap<String, TrianglelistDedupedTextureProperty> =
                HashMap::new();

            for trianglelist_texture_file_name in trianglelist_texture_file_name_list {
                let hash =
                    SSMTStringUtils::get_file_hash_from_file_name(&trianglelist_texture_file_name);

                let fa_log_deduped_filename = {
                    let deduped = fa.log.get_deduped_filename(&trianglelist_texture_file_name);
                    if deduped.trim().is_empty() {
                        String::new()
                    } else {
                        format!("{}_{}", hash, deduped)
                    }
                };

                // Try to discover deduped file from FrameAnalysis/deduped folder by hash.
                let fa_data_deduped_filename = {
                    let mut out = String::new();
                    let deduped_dir = Path::new(&fa.folder_path).join("deduped");
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

            let save_json_file_path = drawib_folder.join("TrianglelistDedupedFileName.json");
            let json_model = TrianglelistDedupedFileNameJson::from_map(trianglelist_deduped_map);
            if let Err(e) = json_model.save_to_file(&save_json_file_path) {
                crate::extract_error!(
                    "Failed to write {}: {}",
                    save_json_file_path.to_string_lossy(),
                    e
                );
            }
        }
    }

    /// Generate `DrawIB-Component.json` in the LOD workspace directory.
    ///
    /// Scans the LOD directory for extracted submesh folders (named
    /// `{draw_ib}-{index_count}-{first_index}`), groups them by DrawIB,
    /// sorts by FirstIndex, and assigns sequential component numbers (0, 1, 2...).
    ///
    /// A folder may carry extra trailing segments when one DrawIB is
    /// pre-skinned several times in a single frame (e.g.
    /// `{draw_ib}-{index_count}-{first_index}-{pose_draw}`). The DrawIB is
    /// always the leading segment and FirstIndex is always the last one, so the
    /// name is split from both ends; names whose FirstIndex is not numeric or
    /// that lack a DrawIB are skipped.
    pub fn generate_draw_ib_component_json(lod_workspace_path: &str) {
        let lod_path = Path::new(lod_workspace_path);
        if !lod_path.exists() {
            crate::extract_error!("LOD workspace path does not exist: {}", lod_workspace_path);
            return;
        }

        // Group submesh folders by DrawIB hash: (folder name, first index)
        let mut draw_ib_submesh_map: HashMap<String, Vec<(String, u64)>> = HashMap::new();

        let entries = match fs::read_dir(lod_path) {
            Ok(entries) => entries,
            Err(e) => {
                crate::extract_error!(
                    "Failed to read LOD workspace directory {}: {}",
                    lod_workspace_path,
                    e
                );
                return;
            }
        };

        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !file_type.is_dir() {
                continue;
            }

            let folder_name = entry.file_name();
            let folder_name_str = match folder_name.to_str() {
                Some(s) => s.to_string(),
                None => continue,
            };

            // Parse folder name: DrawIB is the leading segment, FirstIndex the trailing one.
            let mut parts = folder_name_str.splitn(2, '-');
            let draw_ib = match parts.next() {
                Some(value) if !value.is_empty() => value.to_string(),
                _ => continue,
            };
            // The trailing segment may be preceded by extra ones (see doc comment).
            let first_index_str = folder_name_str.rsplit('-').next().unwrap_or_default();
            let first_index = match first_index_str.parse::<u64>() {
                Ok(value) => value,
                Err(_) => continue,
            };

            draw_ib_submesh_map
                .entry(draw_ib)
                .or_default()
                .push((folder_name_str, first_index));
        }

        if draw_ib_submesh_map.is_empty() {
            crate::extract_log!(
                "No extracted submesh folders found in {}",
                lod_workspace_path
            );
            return;
        }

        // Sort each DrawIB's submeshes by FirstIndex (then by folder name for a
        // stable order — several pre-skinned components share FirstIndex 0),
        // and assign component numbers.
        let mut component_map: HashMap<String, BTreeMap<String, String>> = HashMap::new();
        for (draw_ib, mut submesh_list) in draw_ib_submesh_map {
            submesh_list
                .sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));

            let mut component_dict: BTreeMap<String, String> = BTreeMap::new();
            for (component_index, (submesh_name, _)) in submesh_list.iter().enumerate() {
                component_dict.insert(component_index.to_string(), submesh_name.clone());
            }
            component_map.insert(draw_ib, component_dict);
        }

        // Write JSON file
        let json_path = lod_path.join("DrawIB-Component.json");
        let content = match serde_json::to_string_pretty(&component_map) {
            Ok(c) => c,
            Err(e) => {
                crate::extract_error!("Failed to serialize DrawIB-Component.json: {}", e);
                return;
            }
        };

        if let Err(e) = fs::write(&json_path, &content) {
            crate::extract_error!("Failed to write {}: {}", json_path.to_string_lossy(), e);
        } else {
            crate::extract_log!("Generated {}", json_path.to_string_lossy());
        }
    }

    /// Restore the GIMI import representation written by builds that mistook
    /// an IB's unique-vertex set for a contiguous VB slice.  GIMI components
    /// share their GPU vertex buffers, so the importer must receive the full
    /// buffer (the baseline representation uses zero range values).
    ///
    /// The migration is deliberately restricted to JSON files carrying the
    /// new DrawCallIndexList metadata, which identifies the affected builds.
    /// It does not alter Blender's Import.json data-type selections.
    pub fn reset_gimi_vertex_ranges_for_import(lod_workspace_path: &str) -> Result<usize, String> {
        let lod_path = Path::new(lod_workspace_path);
        let submesh_entries = fs::read_dir(lod_path).map_err(|error| {
            format!(
                "Failed to read LOD workspace directory {}: {}",
                lod_workspace_path, error
            )
        })?;

        let mut repaired_count = 0usize;
        for submesh_entry in submesh_entries.flatten() {
            if !submesh_entry
                .file_type()
                .map(|kind| kind.is_dir())
                .unwrap_or(false)
            {
                continue;
            }

            let data_type_entries = match fs::read_dir(submesh_entry.path()) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for data_type_entry in data_type_entries.flatten() {
                if !data_type_entry
                    .file_type()
                    .map(|kind| kind.is_dir())
                    .unwrap_or(false)
                {
                    continue;
                }
                if !data_type_entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("TYPE_")
                {
                    continue;
                }

                let json_entries = match fs::read_dir(data_type_entry.path()) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                for json_entry in json_entries.flatten() {
                    let json_path = json_entry.path();
                    if !json_entry
                        .file_type()
                        .map(|kind| kind.is_file())
                        .unwrap_or(false)
                        || json_path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            != Some("json")
                    {
                        continue;
                    }

                    let content = match fs::read_to_string(&json_path) {
                        Ok(content) => content,
                        Err(_) => continue,
                    };
                    let mut value: serde_json::Value = match serde_json::from_str(&content) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let Some(object) = value.as_object_mut() else {
                        continue;
                    };
                    let is_gimi = object
                        .get("GamePreset")
                        .and_then(|item| item.as_str())
                        .is_some_and(|preset| preset.eq_ignore_ascii_case("GIMI"));
                    if !is_gimi || !object.contains_key("DrawCallIndexList") {
                        continue;
                    }

                    let has_nonzero_range =
                        ["VertexOffset", "VertexCount", "IndexOffset", "IndexCount"]
                            .iter()
                            .any(|key| {
                                object
                                    .get(*key)
                                    .and_then(|item| item.as_i64())
                                    .unwrap_or_default()
                                    != 0
                            });
                    if !has_nonzero_range {
                        continue;
                    }

                    for key in ["VertexOffset", "VertexCount", "IndexOffset", "IndexCount"] {
                        object.insert(key.to_string(), serde_json::Value::from(0));
                    }
                    let content = serde_json::to_string_pretty(&value).map_err(|error| {
                        format!(
                            "Failed to serialize {}: {}",
                            json_path.to_string_lossy(),
                            error
                        )
                    })?;
                    fs::write(&json_path, content).map_err(|error| {
                        format!("Failed to write {}: {}", json_path.to_string_lossy(), error)
                    })?;
                    repaired_count += 1;
                }
            }
        }

        Ok(repaired_count)
    }
}
