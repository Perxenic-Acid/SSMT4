use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::helper::mark_texture_helper::MarkTextureHelper;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMergePreview {
    pub conflicting_hashes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMergeLod {
    pub name: String,
    pub first_lod_name: Option<String>,
    pub second_lod_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMergeResult {
    pub workspace_name: String,
    pub lods: Vec<WorkspaceMergeLod>,
    pub copied_file_count: usize,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMergeMode {
    Chain,
    Zip,
}

fn workspace_path(base: &Path, name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name.trim());
    if name.trim().is_empty()
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err("Invalid workspace name".to_string());
    }

    let result = base.join(path);
    if !result.is_dir() {
        return Err(format!("Workspace does not exist: {}", name.trim()));
    }
    Ok(result)
}

fn lod_directories(workspace: &Path) -> Result<Vec<String>, String> {
    let mut lods = fs::read_dir(workspace)
        .map_err(|error| format!("Failed to read workspace: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.eq_ignore_ascii_case("Config"))
        .collect::<BTreeSet<_>>();

    let tabs_path = workspace.join("Config").join("WorkPageTabs.json");
    if let Ok(content) = fs::read_to_string(tabs_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(tabs) = json.get("tabs").and_then(serde_json::Value::as_array) {
                for tab in tabs {
                    let Some(name) = tab.get("name").and_then(serde_json::Value::as_str) else {
                        continue;
                    };
                    let path = Path::new(name);
                    if path.components().count() == 1
                        && matches!(path.components().next(), Some(Component::Normal(_)))
                        && !name.eq_ignore_ascii_case("Config")
                    {
                        lods.insert(name.to_string());
                    }
                }
            }
        }
    }

    Ok(lods.into_iter().collect())
}

fn hash_from_file_name(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if !bytes[start].is_ascii_hexdigit() {
            start += 1;
            continue;
        }

        let end = bytes[start..]
            .iter()
            .position(|byte| !byte.is_ascii_hexdigit())
            .map(|length| start + length)
            .unwrap_or(bytes.len());
        if end - start == 8 {
            return Some(name[start..end].to_ascii_lowercase());
        }
        start = end + 1;
    }
    None
}

fn collect_hashes_from_text(text: &str, output: &mut HashSet<String>) {
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if !bytes[start].is_ascii_hexdigit() {
            start += 1;
            continue;
        }
        let end = bytes[start..]
            .iter()
            .position(|byte| !byte.is_ascii_hexdigit())
            .map(|length| start + length)
            .unwrap_or(bytes.len());
        if end - start == 8 {
            output.insert(text[start..end].to_ascii_lowercase());
        }
        start = end + 1;
    }
}

fn collect_draw_ib_hashes_from_json(value: &serde_json::Value, output: &mut HashSet<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_draw_ib_hashes_from_json(item, output);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                let normalized_key = key.replace(['_', '-'], "").to_ascii_lowercase();
                if normalized_key == "drawib" {
                    if let serde_json::Value::String(hash) = value {
                        collect_hashes_from_text(hash, output);
                    }
                }
                collect_draw_ib_hashes_from_json(value, output);
            }
        }
        _ => {}
    }
}

fn collect_draw_ib_hashes(root: &Path, output: &mut HashSet<String>) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|error| format!("Failed to read workspace files: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Failed to read workspace entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to read file type: {error}"))?;
        if file_type.is_dir() {
            collect_draw_ib_hashes(&entry.path(), output)?;
        } else if file_type.is_file() {
            if source_path_extension_is_json(&entry.path()) {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        collect_draw_ib_hashes_from_json(&json, output);
                    }
                }
            }
        }
    }
    Ok(())
}

fn source_path_extension_is_json(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn is_draw_ib_component_map(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("DrawIB-Component.json"))
}

fn is_object_map_metadata(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("ComponentName_DrawCallIndexList.json")
                || name.eq_ignore_ascii_case("TrianglelistDedupedFileName.json")
        })
}

fn is_draw_ib_config(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Config.json"))
}

fn read_json_object_if_present(
    path: &Path,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    if !path.is_file() {
        return Ok(serde_json::Map::new());
    }
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read merge metadata {}: {error}",
            path.to_string_lossy()
        )
    })?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Invalid merge metadata {}: {error}", path.to_string_lossy()))
}

fn source_submesh_is_selected(
    submesh_name: &str,
    source_key: &str,
    preferences: &HashMap<String, String>,
    conflicting_hashes: &HashSet<String>,
) -> bool {
    match hash_from_file_name(submesh_name) {
        Some(hash) => {
            !conflicting_hashes.contains(&hash)
                || preferences.get(&hash).map(String::as_str) == Some(source_key)
        }
        None => true,
    }
}

/// The legacy trianglelist metadata is indexed only by DrawCall filename.
/// DrawCall indexes repeat across separate captures, so a merged LOD needs a
/// submesh-scoped copy to keep its face/body texture sets distinct.
fn write_submesh_trianglelist_metadata(
    target_lod: &Path,
    sources: &[(Option<PathBuf>, &str)],
    preferences: &HashMap<String, String>,
    conflicting_hashes: &HashSet<String>,
) -> Result<(), String> {
    let mut scoped_map: HashMap<String, serde_json::Map<String, serde_json::Value>> =
        HashMap::new();

    for (source_lod, source_key) in sources {
        let Some(source_lod) = source_lod else {
            continue;
        };
        let component_map =
            read_json_object_if_present(&source_lod.join("ComponentName_DrawCallIndexList.json"))?;
        let trianglelist_map =
            read_json_object_if_present(&source_lod.join("TrianglelistDedupedFileName.json"))?;

        for (submesh_name, draw_calls) in component_map {
            if !target_lod.join(&submesh_name).is_dir()
                || !source_submesh_is_selected(
                    &submesh_name,
                    source_key,
                    preferences,
                    conflicting_hashes,
                )
            {
                continue;
            }
            let Some(draw_calls) = draw_calls.as_array() else {
                continue;
            };
            let draw_calls = draw_calls
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|draw_call| !draw_call.is_empty())
                .collect::<Vec<_>>();
            if draw_calls.is_empty() {
                continue;
            }

            let target_textures = scoped_map.entry(submesh_name).or_default();
            for (texture_name, property) in &trianglelist_map {
                if draw_calls
                    .iter()
                    .any(|draw_call| texture_name.starts_with(draw_call))
                {
                    target_textures
                        .entry(texture_name.clone())
                        .or_insert_with(|| property.clone());
                }
            }
        }
    }

    if scoped_map.is_empty() {
        return Ok(());
    }
    let content = serde_json::to_string_pretty(&scoped_map)
        .map_err(|error| format!("Failed to serialize submesh texture metadata: {error}"))?;
    fs::write(
        target_lod.join("SubMeshTrianglelistDedupedFileName.json"),
        content,
    )
    .map_err(|error| format!("Failed to write submesh texture metadata: {error}"))
}

fn write_component_aliases(
    target_lod: &Path,
    sources: &[(Option<PathBuf>, &str)],
    preferences: &HashMap<String, String>,
    conflicting_hashes: &HashSet<String>,
) -> Result<(), String> {
    use crate::workspace::component_aliases::{
        read_component_aliases, ComponentAliases, COMPONENT_ALIASES_FILE,
    };
    let mut merged = ComponentAliases::new();
    for (lod, source_key) in sources {
        let Some(lod) = lod else { continue };
        for (key, alias) in read_component_aliases(lod)? {
            // 别名跟随被选中的实际资源，不能把另一个工作空间同 IB 的标签串过来。
            if target_lod.join(&key).is_dir()
                && source_submesh_is_selected(&key, source_key, preferences, conflicting_hashes)
            {
                merged.entry(key).or_insert(alias);
            }
        }
    }
    if !merged.is_empty() {
        fs::write(
            target_lod.join(COMPONENT_ALIASES_FILE),
            serde_json::to_vec_pretty(&merged).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("保存合并后的 Component 别名失败: {e}"))?;
    }
    Ok(())
}

fn merge_object_map_metadata(source: &Path, target: &Path) -> Result<(), String> {
    let source_content = fs::read_to_string(source)
        .map_err(|error| format!("Failed to read merge metadata: {error}"))?;
    let target_content = fs::read_to_string(target)
        .map_err(|error| format!("Failed to read merge metadata: {error}"))?;
    let source_map =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&source_content)
            .map_err(|error| {
                format!(
                    "Invalid merge metadata {}: {error}",
                    source.to_string_lossy()
                )
            })?;
    let mut target_map =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&target_content)
            .map_err(|error| {
                format!(
                    "Invalid merge metadata {}: {error}",
                    target.to_string_lossy()
                )
            })?;

    for (key, value) in source_map {
        target_map.entry(key).or_insert(value);
    }

    let content = serde_json::to_string_pretty(&target_map)
        .map_err(|error| format!("Failed to serialize merged metadata: {error}"))?;
    fs::write(target, content).map_err(|error| format!("Failed to write merged metadata: {error}"))
}

fn merge_draw_ib_config(
    source: &Path,
    target: &Path,
    source_key: &str,
    preferences: &HashMap<String, String>,
) -> Result<(), String> {
    let source_content = fs::read_to_string(source)
        .map_err(|error| format!("Failed to read DrawIB config: {error}"))?;
    let target_content = fs::read_to_string(target)
        .map_err(|error| format!("Failed to read DrawIB config: {error}"))?;
    let source_entries =
        serde_json::from_str::<Vec<serde_json::Value>>(&source_content).map_err(|error| {
            format!(
                "Invalid DrawIB config {}: {error}",
                source.to_string_lossy()
            )
        })?;
    let mut target_entries = serde_json::from_str::<Vec<serde_json::Value>>(&target_content)
        .map_err(|error| {
            format!(
                "Invalid DrawIB config {}: {error}",
                target.to_string_lossy()
            )
        })?;

    let mut entry_indexes = HashMap::new();
    for (index, entry) in target_entries.iter().enumerate() {
        if let Some(draw_ib) = entry.get("DrawIB").and_then(serde_json::Value::as_str) {
            entry_indexes.insert(draw_ib.trim().to_ascii_lowercase(), index);
        }
    }

    for entry in source_entries {
        let Some(draw_ib) = entry.get("DrawIB").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let draw_ib = draw_ib.trim().to_ascii_lowercase();
        if draw_ib.is_empty() {
            continue;
        }
        if let Some(index) = entry_indexes.get(&draw_ib) {
            if preferences.get(&draw_ib).map(String::as_str) == Some(source_key) {
                target_entries[*index] = entry;
            }
        } else {
            entry_indexes.insert(draw_ib, target_entries.len());
            target_entries.push(entry);
        }
    }

    let content = serde_json::to_string_pretty(&target_entries)
        .map_err(|error| format!("Failed to serialize merged DrawIB config: {error}"))?;
    fs::write(target, content)
        .map_err(|error| format!("Failed to write merged DrawIB config: {error}"))
}

fn workspace_draw_ib_hashes(workspace: &Path) -> Result<HashSet<String>, String> {
    let mut hashes = HashSet::new();
    collect_draw_ib_hashes(workspace, &mut hashes)?;
    Ok(hashes)
}

#[tauri::command]
pub fn workspace_merge_preview(
    workspace_base: String,
    first_workspace_name: String,
    second_workspace_name: String,
) -> Result<WorkspaceMergePreview, String> {
    let base = PathBuf::from(workspace_base);
    let first = workspace_path(&base, &first_workspace_name)?;
    let second = workspace_path(&base, &second_workspace_name)?;
    if first == second {
        return Err("Select two different workspaces".to_string());
    }

    let first_hashes = workspace_draw_ib_hashes(&first)?;
    let second_hashes = workspace_draw_ib_hashes(&second)?;
    let conflicting_hashes = first_hashes
        .intersection(&second_hashes)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(WorkspaceMergePreview { conflicting_hashes })
}

fn copy_lod(
    source: &Path,
    target: &Path,
    source_key: &str,
    preferences: &HashMap<String, String>,
    conflicting_hashes: &HashSet<String>,
    copied_paths: &mut HashSet<PathBuf>,
    copied_file_count: &mut usize,
) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| format!("Failed to create merged LOD: {error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("Failed to read LOD: {error}"))? {
        let entry = entry.map_err(|error| format!("Failed to read LOD entry: {error}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to read file type: {error}"))?;
        if file_type.is_dir() {
            copy_lod(
                &source_path,
                &target_path,
                source_key,
                preferences,
                conflicting_hashes,
                copied_paths,
                copied_file_count,
            )?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        // This map is derived from the extracted submesh folders. It must not
        // be carried over from either source, or a zip merge hides the other
        // source's components in the post-processing page.
        if is_draw_ib_component_map(&source_path)
            || entry.file_name() == crate::workspace::component_aliases::COMPONENT_ALIASES_FILE
        {
            continue;
        }

        if let Some(hash) = hash_from_file_name(&entry.file_name().to_string_lossy()) {
            if conflicting_hashes.contains(&hash)
                && preferences.get(&hash).map(String::as_str) != Some(source_key)
            {
                continue;
            }
        }

        if !copied_paths.insert(target_path.clone()) {
            if is_object_map_metadata(&source_path) {
                merge_object_map_metadata(&source_path, &target_path)?;
            } else if is_draw_ib_config(&source_path) {
                merge_draw_ib_config(&source_path, &target_path, source_key, preferences)?;
            }
            continue;
        }
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create output folder: {error}"))?;
        }
        fs::copy(&source_path, &target_path)
            .map_err(|error| format!("Failed to copy workspace file: {error}"))?;
        *copied_file_count += 1;
    }
    Ok(())
}

fn copy_lod_if_present(
    source: &Path,
    target: &Path,
    source_key: &str,
    preferences: &HashMap<String, String>,
    conflicting_hashes: &HashSet<String>,
    copied_paths: &mut HashSet<PathBuf>,
    copied_file_count: &mut usize,
) -> Result<(), String> {
    if source.is_dir() {
        return copy_lod(
            source,
            target,
            source_key,
            preferences,
            conflicting_hashes,
            copied_paths,
            copied_file_count,
        );
    }
    fs::create_dir_all(target).map_err(|error| format!("Failed to create merged LOD: {error}"))
}

#[tauri::command]
pub fn workspace_merge(
    workspace_base: String,
    first_workspace_name: String,
    second_workspace_name: String,
    output_workspace_name: String,
    mode: WorkspaceMergeMode,
    hash_preferences: HashMap<String, String>,
) -> Result<WorkspaceMergeResult, String> {
    let base = PathBuf::from(workspace_base);
    let first = workspace_path(&base, &first_workspace_name)?;
    let second = workspace_path(&base, &second_workspace_name)?;
    if first == second {
        return Err("Select two different workspaces".to_string());
    }

    let output_name = output_workspace_name.trim();
    let output_component = Path::new(output_name);
    if output_name.is_empty()
        || output_component.components().count() != 1
        || !matches!(
            output_component.components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("Invalid output workspace name".to_string());
    }
    let output = base.join(output_component);
    if output.exists() {
        return Err(format!("Workspace already exists: {output_name}"));
    }

    let first_hashes = workspace_draw_ib_hashes(&first)?;
    let second_hashes = workspace_draw_ib_hashes(&second)?;
    let conflicting_hashes = first_hashes
        .intersection(&second_hashes)
        .cloned()
        .collect::<HashSet<_>>();
    for hash in &conflicting_hashes {
        match hash_preferences.get(hash).map(String::as_str) {
            Some("first") | Some("second") => {}
            _ => return Err(format!("Choose a source for hash {hash}")),
        }
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_nanos();
    let temporary = base.join(format!(".ssmt-merge-{nonce}"));
    fs::create_dir(&temporary)
        .map_err(|error| format!("Failed to create merge workspace: {error}"))?;

    let build_result = (|| {
        let first_lods = lod_directories(&first)?;
        let second_lods = lod_directories(&second)?;
        let mut lods = Vec::new();
        let mut copied_paths = HashSet::new();
        let mut copied_file_count = 0;

        match mode {
            WorkspaceMergeMode::Chain => {
                for (index, source_lod) in first_lods.iter().enumerate() {
                    let name = format!("LOD{index}");
                    copy_lod_if_present(
                        &first.join(source_lod),
                        &temporary.join(&name),
                        "first",
                        &hash_preferences,
                        &conflicting_hashes,
                        &mut copied_paths,
                        &mut copied_file_count,
                    )?;
                    lods.push(WorkspaceMergeLod {
                        name,
                        first_lod_name: Some(source_lod.clone()),
                        second_lod_name: None,
                    });
                }
                let offset = lods.len();
                for (index, source_lod) in second_lods.iter().enumerate() {
                    let name = format!("LOD{}", offset + index);
                    copy_lod_if_present(
                        &second.join(source_lod),
                        &temporary.join(&name),
                        "second",
                        &hash_preferences,
                        &conflicting_hashes,
                        &mut copied_paths,
                        &mut copied_file_count,
                    )?;
                    lods.push(WorkspaceMergeLod {
                        name,
                        first_lod_name: None,
                        second_lod_name: Some(source_lod.clone()),
                    });
                }
            }
            WorkspaceMergeMode::Zip => {
                let names = first_lods
                    .iter()
                    .chain(second_lods.iter())
                    .cloned()
                    .collect::<BTreeSet<_>>();
                for name in names {
                    let first_lod_name = first_lods
                        .iter()
                        .find(|lod| lod.eq_ignore_ascii_case(&name))
                        .cloned();
                    let second_lod_name = second_lods
                        .iter()
                        .find(|lod| lod.eq_ignore_ascii_case(&name))
                        .cloned();
                    if let Some(source_lod) = &first_lod_name {
                        copy_lod_if_present(
                            &first.join(source_lod),
                            &temporary.join(&name),
                            "first",
                            &hash_preferences,
                            &conflicting_hashes,
                            &mut copied_paths,
                            &mut copied_file_count,
                        )?;
                    }
                    if let Some(source_lod) = &second_lod_name {
                        copy_lod_if_present(
                            &second.join(source_lod),
                            &temporary.join(&name),
                            "second",
                            &hash_preferences,
                            &conflicting_hashes,
                            &mut copied_paths,
                            &mut copied_file_count,
                        )?;
                    }
                    lods.push(WorkspaceMergeLod {
                        name,
                        first_lod_name,
                        second_lod_name,
                    });
                }
            }
        }
        for lod in &lods {
            let lod_path = temporary.join(&lod.name);
            write_component_aliases(
                &lod_path,
                &[
                    (
                        lod.first_lod_name.as_ref().map(|name| first.join(name)),
                        "first",
                    ),
                    (
                        lod.second_lod_name.as_ref().map(|name| second.join(name)),
                        "second",
                    ),
                ],
                &hash_preferences,
                &conflicting_hashes,
            )?;
            write_submesh_trianglelist_metadata(
                &lod_path,
                &[
                    (
                        lod.first_lod_name.as_ref().map(|name| first.join(name)),
                        "first",
                    ),
                    (
                        lod.second_lod_name.as_ref().map(|name| second.join(name)),
                        "second",
                    ),
                ],
                &hash_preferences,
                &conflicting_hashes,
            )?;
            MarkTextureHelper::generate_draw_ib_component_json(&lod_path.to_string_lossy());
        }

        Ok::<_, String>((lods, copied_file_count))
    })();

    let (lods, copied_file_count) = match build_result {
        Ok(result) => result,
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
    };
    fs::rename(&temporary, &output).map_err(|error| {
        let _ = fs::remove_dir_all(&temporary);
        format!("Failed to finalize merged workspace: {error}")
    })?;

    Ok(WorkspaceMergeResult {
        workspace_name: output_name.to_string(),
        lods,
        copied_file_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_merge_keeps_the_selected_hash_source() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is available")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ssmt-workspace-merge-test-{nonce}"));
        let first_lod = base.join("first").join("LOD0");
        let second_lod = base.join("second").join("LOD0");
        fs::create_dir_all(&first_lod).expect("create first workspace");
        fs::create_dir_all(&second_lod).expect("create second workspace");
        fs::write(
            first_lod.join("DrawIBConfig.json"),
            r#"[{"DrawIB":"aaaaaaaa"}]"#,
        )
        .expect("write first DrawIB config");
        fs::write(
            second_lod.join("DrawIBConfig.json"),
            r#"[{"DrawIB":"aaaaaaaa"}]"#,
        )
        .expect("write second DrawIB config");
        fs::write(first_lod.join("first_aaaaaaaa.buf"), b"first").expect("write first file");
        fs::write(second_lod.join("second_aaaaaaaa.buf"), b"second").expect("write second file");
        fs::write(second_lod.join("second_bbbbbbbb.buf"), b"unique").expect("write unique file");

        fs::create_dir_all(first_lod.join("aaaaaaaa-3-0")).unwrap();
        fs::create_dir_all(second_lod.join("aaaaaaaa-3-0")).unwrap();
        fs::write(
            first_lod.join("ComponentAliases.json"),
            r#"{"aaaaaaaa-3-0":"旧面部"}"#,
        )
        .unwrap();
        fs::write(
            second_lod.join("ComponentAliases.json"),
            r#"{"aaaaaaaa-3-0":"选中面部"}"#,
        )
        .unwrap();
        let preview = workspace_merge_preview(
            base.to_string_lossy().into_owned(),
            "first".to_string(),
            "second".to_string(),
        )
        .expect("preview merge");
        assert_eq!(preview.conflicting_hashes, vec!["aaaaaaaa"]);

        let result = workspace_merge(
            base.to_string_lossy().into_owned(),
            "first".to_string(),
            "second".to_string(),
            "merged".to_string(),
            WorkspaceMergeMode::Zip,
            HashMap::from([("aaaaaaaa".to_string(), "second".to_string())]),
        )
        .expect("merge workspaces");

        assert_eq!(result.lods.len(), 1);
        assert!(!base.join("merged/LOD0/first_aaaaaaaa.buf").exists());
        assert_eq!(
            fs::read(base.join("merged/LOD0/second_aaaaaaaa.buf")).expect("read selected file"),
            b"second"
        );
        assert!(base.join("merged/LOD0/second_bbbbbbbb.buf").exists());
        assert_eq!(
            crate::workspace::component_aliases::read_component_aliases(&base.join("merged/LOD0"))
                .unwrap()["aaaaaaaa-3-0"],
            "选中面部"
        );
        fs::remove_dir_all(base).expect("remove test workspace");
    }

    #[test]
    fn zip_merge_unions_post_process_metadata_and_regenerates_components() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is available")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("ssmt-workspace-merge-metadata-test-{nonce}"));
        let first_lod = base.join("first").join("LOD0");
        let second_lod = base.join("second").join("LOD0");
        fs::create_dir_all(first_lod.join("aaaaaaaa-3-0")).expect("create first submesh");
        fs::create_dir_all(second_lod.join("bbbbbbbb-3-0")).expect("create second submesh");

        fs::write(
            first_lod.join("Config.json"),
            r#"[{"DrawIB":"aaaaaaaa","Alias":"First"}]"#,
        )
        .expect("write first config");
        fs::write(
            second_lod.join("Config.json"),
            r#"[{"DrawIB":"bbbbbbbb","Alias":"Second"}]"#,
        )
        .expect("write second config");
        fs::write(
            first_lod.join("ComponentAliases.json"),
            r#"{"aaaaaaaa-3-0":"脸"}"#,
        )
        .unwrap();
        fs::write(
            second_lod.join("ComponentAliases.json"),
            r#"{"bbbbbbbb-3-0":"衣服"}"#,
        )
        .unwrap();
        fs::write(
            first_lod.join("ComponentName_DrawCallIndexList.json"),
            r#"{"aaaaaaaa-3-0":["000001"]}"#,
        )
        .expect("write first component metadata");
        fs::write(
            second_lod.join("ComponentName_DrawCallIndexList.json"),
            r#"{"bbbbbbbb-3-0":["000001"]}"#,
        )
        .expect("write second component metadata");
        fs::write(
            first_lod.join("TrianglelistDedupedFileName.json"),
            r#"{"000001-ps-t0=deadbeef.dds":{"FALogDedupedFileName":"first"}}"#,
        )
        .expect("write first texture metadata");
        fs::write(
            second_lod.join("TrianglelistDedupedFileName.json"),
            r#"{"000001-ps-t0=deadbeef.dds":{"FALogDedupedFileName":"second"}}"#,
        )
        .expect("write second texture metadata");

        workspace_merge(
            base.to_string_lossy().into_owned(),
            "first".to_string(),
            "second".to_string(),
            "merged".to_string(),
            WorkspaceMergeMode::Zip,
            HashMap::new(),
        )
        .expect("merge workspaces");

        let merged_lod = base.join("merged").join("LOD0");
        let aliases =
            crate::workspace::component_aliases::read_component_aliases(&merged_lod).unwrap();
        assert_eq!(aliases["aaaaaaaa-3-0"], "脸");
        assert_eq!(aliases["bbbbbbbb-3-0"], "衣服");
        let component_map: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(merged_lod.join("ComponentName_DrawCallIndexList.json"))
                .expect("read merged component metadata"),
        )
        .expect("parse merged component metadata");
        assert!(component_map.get("aaaaaaaa-3-0").is_some());
        assert!(component_map.get("bbbbbbbb-3-0").is_some());

        let texture_map: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(merged_lod.join("TrianglelistDedupedFileName.json"))
                .expect("read merged texture metadata"),
        )
        .expect("parse merged texture metadata");
        assert_eq!(
            texture_map["000001-ps-t0=deadbeef.dds"]["FALogDedupedFileName"],
            serde_json::Value::String("first".to_string())
        );

        let scoped_texture_map: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(merged_lod.join("SubMeshTrianglelistDedupedFileName.json"))
                .expect("read submesh-scoped texture metadata"),
        )
        .expect("parse submesh-scoped texture metadata");
        assert_eq!(
            scoped_texture_map["aaaaaaaa-3-0"]["000001-ps-t0=deadbeef.dds"]["FALogDedupedFileName"],
            serde_json::Value::String("first".to_string())
        );
        assert_eq!(
            scoped_texture_map["bbbbbbbb-3-0"]["000001-ps-t0=deadbeef.dds"]["FALogDedupedFileName"],
            serde_json::Value::String("second".to_string())
        );

        let draw_ib_component_map: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(merged_lod.join("DrawIB-Component.json"))
                .expect("read regenerated DrawIB component map"),
        )
        .expect("parse regenerated DrawIB component map");
        assert_eq!(
            draw_ib_component_map["aaaaaaaa"]["0"],
            serde_json::Value::String("aaaaaaaa-3-0".to_string())
        );
        assert_eq!(
            draw_ib_component_map["bbbbbbbb"]["0"],
            serde_json::Value::String("bbbbbbbb-3-0".to_string())
        );

        let config: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(merged_lod.join("Config.json")).expect("read merged config"),
        )
        .expect("parse merged config");
        assert_eq!(config.as_array().expect("config array").len(), 2);
        fs::remove_dir_all(base).expect("remove test workspace");
    }
}
