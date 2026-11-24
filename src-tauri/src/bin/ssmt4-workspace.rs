use serde::Deserialize;
use serde_json::{json, Value};
use ssmt4_lib::common::frame_analysis::frameanalysis::FrameAnalysis;
use ssmt4_lib::extract_new::extract_services::{
    ExtractNewService, FullExtractDataTypeFilter,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
struct Request {
    workspace: PathBuf,
    game: String,
    lods: Vec<Lod>,
}

#[derive(Deserialize)]
struct Lod {
    name: String,
    frame: PathBuf,
    #[serde(default)]
    draw_ibs: Vec<DrawIb>,
    full_filter: Option<String>,
    #[serde(default)]
    textures: Vec<Texture>,
}

#[derive(Deserialize)]
struct DrawIb {
    hash: String,
    #[serde(default)]
    alias: String,
}

#[derive(Deserialize)]
struct Texture {
    submesh: String,
    capture_name: Option<String>,
    name: String,
    hash: String,
    slot: String,
    mark_type: String,
    face_sdf_channel: Option<String>,
}

fn component(value: &str) -> Result<&str> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || value.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || value.chars().any(char::is_control)
    {
        return Err(format!("Invalid path component: {value:?}").into());
    }
    Ok(value)
}

fn hash(value: &str) -> Result<String> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("Invalid 8-digit hash: {value:?}").into());
    }
    Ok(value.to_ascii_lowercase())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn mark_textures(lod: &Path, marks: &[Texture]) -> Result<()> {
    if marks.is_empty() {
        return Ok(());
    }
    let dictionary: Value =
        serde_json::from_slice(&fs::read(lod.join("TrianglelistDedupedFileName.json"))?)?;
    for mark in marks {
        let submesh = component(&mark.submesh)?;
        let name = component(&mark.name)?;
        let texture_hash = hash(&mark.hash)?;
        let deduped = if let Some(capture_name) = &mark.capture_name {
            dictionary[capture_name]["FALogDedupedFileName"]
                .as_str()
                .ok_or_else(|| format!("Texture was not extracted: {capture_name}"))?
        } else {
            let key = format!("-{}={texture_hash}-", mark.slot.to_ascii_lowercase());
            let matches: std::collections::BTreeSet<_> = dictionary
                .as_object()
                .ok_or("Invalid triangle-list texture index")?
                .iter()
                .filter(|(name, _)| name.to_ascii_lowercase().contains(&key))
                .filter_map(|(_, entry)| entry["FALogDedupedFileName"].as_str())
                .filter(|name| !name.is_empty())
                .collect();
            if matches.len() != 1 {
                return Err(format!(
                    "Texture {} in {} has {} matching files; specify capture_name",
                    texture_hash,
                    mark.slot,
                    matches.len()
                )
                .into());
            }
            *matches.first().unwrap()
        };
        let source = lod.join("DedupedTextures").join(component(deduped)?);
        let suffix = source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("dds");
        let filename = format!("{submesh}-{name}-{texture_hash}.{suffix}");
        let mut markup = json!({
            "MarkName": name, "MarkHash": texture_hash, "MarkSlot": mark.slot,
            "MarkType": mark.mark_type, "MarkFileName": filename, "MarkDedupedFileName": deduped,
        });
        if let Some(channel) = &mark.face_sdf_channel {
            markup["FaceSDFChannel"] = json!(channel);
        }
        let mut found = false;
        for dir in fs::read_dir(lod.join(submesh))? {
            let dir = dir?.path();
            let path = dir.join(format!("{submesh}.json"));
            if !path.is_file() {
                continue;
            }
            let mut metadata: Value = serde_json::from_slice(&fs::read(&path)?)?;
            if metadata["TextureMarkUpInfoList"].is_null() {
                metadata["TextureMarkUpInfoList"] = json!([]);
            }
            let list = metadata["TextureMarkUpInfoList"]
                .as_array_mut()
                .ok_or("TextureMarkUpInfoList is not an array")?;
            list.retain(|item| {
                !(item["MarkHash"] == texture_hash && item["MarkSlot"] == mark.slot)
            });
            list.push(markup.clone());
            fs::copy(&source, dir.join(&filename))?;
            write_json(&path, &metadata)?;
            found = true;
        }
        if !found {
            return Err(format!("No metadata found for submesh {submesh}").into());
        }
    }
    Ok(())
}

fn run(request: Request) -> Result<()> {
    if !request.workspace.is_absolute() || request.lods.is_empty() {
        return Err("Provide an absolute workspace path and at least one LOD".into());
    }

    let mut names = std::collections::HashSet::new();
    for lod in &request.lods {
        component(&lod.name)?;
        if !names.insert(lod.name.to_ascii_lowercase())
            || !lod.frame.is_absolute()
            || !lod.frame.is_dir()
        {
            return Err(format!("Duplicate LOD or invalid frame directory: {}", lod.name).into());
        }
        if (lod.full_filter.is_none() && lod.draw_ibs.is_empty())
            || (lod.full_filter.is_some() && !lod.draw_ibs.is_empty())
        {
            return Err(format!("{}: specify either draw_ibs or full_filter", lod.name).into());
        }
        for part in &lod.draw_ibs {
            hash(&part.hash)?;
        }
    }

    let workspace = request
        .workspace
        .to_str()
        .ok_or("Workspace path is not Unicode")?;
    fs::create_dir_all(&request.workspace)?;
    let config = request.workspace.join("Config");
    let tab_dir = config.join("Tabs");
    fs::create_dir_all(&tab_dir)?;
    let tabs: Vec<_> = request
        .lods
        .iter()
        .enumerate()
        .map(|(i, lod)| json!({"id": format!("ws-tab-cli-{i}"), "name": lod.name}))
        .collect();
    write_json(
        &config.join("WorkPageTabs.json"),
        &json!({
            "activeTabId": "ws-tab-cli-0", "tabs": tabs,
        }),
    )?;
    for (i, lod) in request.lods.into_iter().enumerate() {
        let output = request.workspace.join(&lod.name);
        fs::create_dir_all(&output)?;
        let rows = lod
            .draw_ibs
            .iter()
            .map(|part| {
                json!({
                    "DrawIB": part.hash.to_ascii_lowercase(), "Alias": part.alias,
                })
            })
            .collect();
        write_json(&output.join("Config.json"), &Value::Array(rows))?;
        let frame = lod
            .frame
            .to_str()
            .ok_or("Frame path is not Unicode")?
            .to_owned();
        let frame_config = json!({
            "frameAnalysisFolderPath": frame,
            "selectedFrameAnalysis": lod.frame.file_name(),
        });
        let mut tab_config = frame_config.clone();
        tab_config["modelRows"] = json!(lod
            .draw_ibs
            .iter()
            .map(|part| {
                json!({"drawIB": part.hash.to_ascii_lowercase(), "aliasName": part.alias})
            })
            .collect::<Vec<_>>());
        if let Some(filter) = &lod.full_filter {
            tab_config["extractPanelTab"] = json!("full");
            tab_config["fullExtractDataTypeFilter"] = json!(filter);
        }
        write_json(&tab_dir.join(format!("ws-tab-cli-{i}.json")), &tab_config)?;
        if i == 0 {
            write_json(&config.join("FrameAnalysisPath.json"), &frame_config)?;
        }
        if !output.join("SkipIBConfig.json").exists() {
            write_json(&output.join("SkipIBConfig.json"), &json!([]))?;
        }
        if !output.join("VSCheckConfig.json").exists() {
            write_json(&output.join("VSCheckConfig.json"), &json!([]))?;
        }
        println!("Extracting {} to {}", lod.name, output.display());
        if let Some(filter) = lod.full_filter {
            let filter = FullExtractDataTypeFilter::from_input(&filter)?;
            if request.game == "ZZMIDX12" {
                ExtractNewService::run_extract_dx12(
                    &frame,
                    output.to_string_lossy().as_ref(),
                    filter,
                    true,
                )?;
            } else {
                let fa = FrameAnalysis::new(&frame)
                    .map_err(|error| format!("Failed to read FrameAnalysis data: {error}"))?;
                ExtractNewService::run_extract(
                    &fa,
                    &request.game,
                    workspace,
                    output.to_string_lossy().as_ref(),
                    filter,
                    true,
                )?;
            }
        } else {
            if request.game == "ZZMIDX12" {
                ExtractNewService::run_extract_dx12(
                    &frame,
                    output.to_string_lossy().as_ref(),
                    FullExtractDataTypeFilter::All,
                    false,
                )?;
            } else {
                let fa = FrameAnalysis::new(&frame)
                    .map_err(|error| format!("Failed to read FrameAnalysis data: {error}"))?;
                ExtractNewService::run_extract(
                    &fa,
                    &request.game,
                    workspace,
                    output.to_string_lossy().as_ref(),
                    FullExtractDataTypeFilter::All,
                    false,
                )?;
            }
        }
        if !lod.textures.is_empty() && !output.join("TrianglelistDedupedFileName.json").is_file() {
            let fa = FrameAnalysis::new(&frame)
                .map_err(|error| format!("Failed to read FrameAnalysis data: {error}"))?;
            ExtractNewService::extract_deduped_textures(
                &fa,
                &request.game,
                workspace,
                output.to_string_lossy().as_ref(),
            )?;
            ExtractNewService::extract_trianglelist_textures(
                &fa,
                &request.game,
                workspace,
                output.to_string_lossy().as_ref(),
            )?;
        }
        mark_textures(&output, &lod.textures)?;
        println!("Completed {}", output.display());
    }
    Ok(())
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("Usage: ssmt4-workspace <request.json>");
        std::process::exit(2);
    };
    if path == "--help" || path == "-h" {
        println!("Usage: ssmt4-workspace <request.json>");
        return;
    }
    if args.next().is_some() {
        eprintln!("Expected exactly one request file");
        std::process::exit(2);
    }
    let result = (|| -> Result<()> {
        let bytes = fs::read(path)?;
        let request =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
        run(request)
    })();
    if let Err(error) = result {
        eprintln!("SSMT4 workspace: {error}");
        std::process::exit(1);
    }
}
