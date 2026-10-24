use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncWriteExt;

const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_VIDEO_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndfieldBackgroundUpdate {
    pub image_path: String,
    pub image_url: String,
    pub image_md5: String,
    pub image_sha256: String,
    pub image_bytes: u64,
    pub video_path: Option<String>,
    pub video_url: Option<String>,
    pub video_sha256: Option<String>,
    pub video_bytes: Option<u64>,
    pub profile_id: String,
    pub source_leveldb: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MainBackground {
    image_url: String,
    image_md5: String,
    video_url: Option<String>,
}

#[derive(Debug, Clone)]
struct Candidate {
    background: MainBackground,
    profile_id: String,
    profile_root: PathBuf,
    source_leveldb: PathBuf,
    modified: SystemTime,
    score: usize,
}

struct StagedAsset {
    path: PathBuf,
    sha256: String,
    md5: String,
}

#[tauri::command]
pub async fn sync_endfield_background(
    app: AppHandle,
    game_name: String,
    previous_image_path: Option<String>,
    previous_video_url: Option<String>,
    previous_video_path: Option<String>,
) -> Result<EndfieldBackgroundUpdate, String> {
    validate_game_directory_name(&game_name)?;
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 LOCALAPPDATA，暂时不能读取鹰角启动器缓存".to_string())?;
    let games_root = local_app_data.join("Games");
    if !games_root.is_dir() {
        return Err("未找到鹰角启动器数据目录。请先打开一次海猫启动器并进入终末地".into());
    }
    let roaming_app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default();
    let candidate = discover_endfield_background(
        &games_root,
        &roaming_app_data.join("Hypergryph"),
    )?;
    let target_dir = app
        .path()
        .resolve("SSMT4GlobalConfigs/Games", BaseDirectory::LocalData)
        .map_err(|error| format!("无法定位 SSMT 游戏配置目录: {error}"))?
        .join(&game_name);
    fs::create_dir_all(&target_dir)
        .map_err(|error| format!("创建终末地背景目录失败: {error}"))?;

    sync_candidate(
        &candidate,
        &target_dir,
        previous_image_path.as_deref(),
        previous_video_url.as_deref(),
        previous_video_path.as_deref(),
    )
    .await
}

fn validate_game_directory_name(game_name: &str) -> Result<(), String> {
    if game_name.is_empty()
        || game_name == "."
        || game_name == ".."
        || game_name.contains(['/', '\\', ':'])
    {
        return Err("无效的 SSMT 游戏目录名".into());
    }
    Ok(())
}

fn is_direct_child(path: &Path, directory: &Path) -> bool {
    path.parent() == Some(directory)
}

fn file_matches_md5(path: &Path, expected_md5: &str) -> bool {
    fs::read(path)
        .map(|bytes| format!("{:x}", md5::compute(bytes)) == expected_md5.to_ascii_lowercase())
        .unwrap_or(false)
}

fn file_is_mp4(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 12];
    file.read_exact(&mut header).is_ok() && &header[4..8] == b"ftyp"
}

fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let bytes = fs::read(path).map_err(|error| format!("读取背景文件失败: {error}"))?;
    Ok((
        format!("{:x}", Sha256::digest(&bytes)),
        bytes.len() as u64,
    ))
}

async fn sync_candidate(
    candidate: &Candidate,
    target_dir: &Path,
    previous_image_path: Option<&str>,
    previous_video_url: Option<&str>,
    previous_video_path: Option<&str>,
) -> Result<EndfieldBackgroundUpdate, String> {
    let image_reuse_path = previous_image_path
        .map(PathBuf::from)
        .filter(|path| is_direct_child(path, target_dir) && file_matches_md5(path, &candidate.background.image_md5));
    let mut changed = false;
    let mut created_image_path = None;

    let (image_path, image_sha256, image_bytes) = if let Some(path) = image_reuse_path {
        let (sha256, bytes) = sha256_file(&path)?;
        (path, sha256, bytes)
    } else {
        let response = reqwest::Client::new()
            .get(&candidate.background.image_url)
            .send()
            .await;
        let image_path = match response {
            Ok(response) => {
                let temp = download_asset(
                    response,
                    target_dir,
                    "endfield-image",
                    "webp",
                    MAX_IMAGE_BYTES,
                )
                .await;
                match temp {
                    Ok(download) if download.md5 == candidate.background.image_md5 => {
                        let extension = image_extension(&candidate.background.image_url);
                        let name = format!(
                            "Background-Endfield-{}.{extension}",
                            candidate.background.image_md5
                        );
                        commit_staged(download, &target_dir.join(name))?
                    }
                    Ok(download) => {
                        let _ = fs::remove_file(&download.path);
                        stage_cached_image(candidate, target_dir)
                            .await
                            .map_err(|cache_error| {
                                format!(
                                    "终末地静态背景 MD5 与启动器元数据不一致; {cache_error}"
                                )
                            })?
                    }
                    Err(download_error) => stage_cached_image(candidate, target_dir)
                        .await
                        .map_err(|cache_error| format!("{download_error}; {cache_error}"))?,
                }
            }
            Err(download_error) => stage_cached_image(candidate, target_dir)
                .await
                .map_err(|cache_error| {
                    format!("下载终末地静态背景失败: {download_error}; {cache_error}")
                })?,
        };
        created_image_path = Some(image_path.clone());
        changed = true;
        let (sha256, bytes) = sha256_file(&image_path)?;
        (image_path, sha256, bytes)
    };

    let (video_path, video_sha256, video_bytes) = if let Some(url) = &candidate.background.video_url {
        validate_video_url(url)?;
        let reuse_path = previous_video_path
            .map(PathBuf::from)
            .filter(|path| {
                previous_video_url == Some(url.as_str())
                    && is_direct_child(path, target_dir)
                    && file_is_mp4(path)
            });
        if let Some(path) = reuse_path {
            let (sha256, bytes) = sha256_file(&path)?;
            (Some(path), Some(sha256), Some(bytes))
        } else {
            let response = reqwest::Client::new()
                .get(url)
                .send()
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    if let Some(path) = &created_image_path {
                        let _ = fs::remove_file(path);
                    }
                    return Err(format!("下载终末地动态背景失败: {error}"));
                }
            };
            let download = match download_asset(
                response,
                target_dir,
                "endfield-video",
                "mp4",
                MAX_VIDEO_BYTES,
            )
            .await
            {
                Ok(download) => download,
                Err(error) => {
                    if let Some(path) = &created_image_path {
                        let _ = fs::remove_file(path);
                    }
                    return Err(error);
                }
            };
            let name = format!(
                "Background-Endfield-{}.mp4",
                &download.sha256[..16.min(download.sha256.len())]
            );
            let path = commit_staged(download, &target_dir.join(name))?;
            changed = true;
            let (sha256, bytes) = sha256_file(&path)?;
            (Some(path), Some(sha256), Some(bytes))
        }
    } else {
        (None, None, None)
    };
    Ok(EndfieldBackgroundUpdate {
        image_path: image_path.to_string_lossy().into_owned(),
        image_url: candidate.background.image_url.clone(),
        image_md5: candidate.background.image_md5.clone(),
        image_sha256,
        image_bytes,
        video_path: video_path.map(|path| path.to_string_lossy().into_owned()),
        video_url: candidate.background.video_url.clone(),
        video_sha256,
        video_bytes,
        profile_id: candidate.profile_id.clone(),
        source_leveldb: candidate.source_leveldb.to_string_lossy().into_owned(),
        changed,
    })
}

async fn download_asset(
    response: reqwest::Response,
    target_dir: &Path,
    stem: &str,
    fallback_extension: &str,
    max_bytes: usize,
) -> Result<StagedAsset, String> {
    if !response.status().is_success() {
        return Err(format!("背景下载失败: HTTP {}", response.status()));
    }
    validate_official_url(response.url().as_str())?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err("背景文件超过大小限制".into());
    }
    let extension = Path::new(response.url().path())
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.len() <= 5)
        .unwrap_or(fallback_extension)
        .to_ascii_lowercase();
    let temp_path = unique_temp_path(target_dir, stem);
    let mut file = tokio::fs::File::create(&temp_path)
        .await
        .map_err(|error| format!("创建背景临时文件失败: {error}"))?;
    let mut stream = response.bytes_stream();
    let mut sha256 = Sha256::new();
    let mut md5 = md5::Context::new();
    let mut total = 0usize;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                return Err(format!("读取背景下载流失败: {error}"));
            }
        };
        total = total.saturating_add(chunk.len());
        if total > max_bytes {
            let _ = fs::remove_file(&temp_path);
            return Err("背景文件超过大小限制".into());
        }
        sha256.update(&chunk);
        md5.consume(&chunk);
        if let Err(error) = file.write_all(&chunk).await {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("写入背景临时文件失败: {error}"));
        }
    }
    if let Err(error) = file.flush().await {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("刷新背景临时文件失败: {error}"));
    }
    if total == 0 || !valid_downloaded_asset(&temp_path, &extension) {
        let _ = fs::remove_file(&temp_path);
        return Err("下载内容不是有效的背景文件".into());
    }
    Ok(StagedAsset {
        path: temp_path,
        sha256: format!("{:x}", sha256.finalize()),
        md5: format!("{:x}", md5.compute()),
    })
}

async fn stage_cached_image(
    candidate: &Candidate,
    target_dir: &Path,
) -> Result<PathBuf, String> {
    let cached = find_cached_image(&candidate.profile_root, &candidate.background.image_md5)
        .ok_or_else(|| "终末地背景下载失败，启动器中也没有可用的同 MD5 缓存".to_string())?;
    let bytes = fs::read(&cached).map_err(|error| format!("读取终末地背景缓存失败: {error}"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("终末地背景缓存超过大小限制".into());
    }
    let extension = image_extension(&candidate.background.image_url);
    let temp_path = unique_temp_path(target_dir, "endfield-cached-image");
    tokio::fs::write(&temp_path, &bytes)
        .await
        .map_err(|error| format!("写入终末地缓存背景失败: {error}"))?;
    let destination = target_dir.join(format!(
        "Background-Endfield-{}.{extension}",
        candidate.background.image_md5
    ));
    commit_staged(
        StagedAsset {
            path: temp_path,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            md5: format!("{:x}", md5::compute(&bytes)),
        },
        &destination,
    )
}

fn commit_staged(staged: StagedAsset, destination: &Path) -> Result<PathBuf, String> {
    if destination.is_file() {
        let (existing_sha256, _) = sha256_file(destination)?;
        if existing_sha256 == staged.sha256 {
            let _ = fs::remove_file(&staged.path);
            return Ok(destination.to_path_buf());
        }
        fs::remove_file(destination)
            .map_err(|error| format!("替换旧背景文件失败: {error}"))?;
    }
    fs::rename(&staged.path, destination).map_err(|error| {
        let _ = fs::remove_file(&staged.path);
        format!("保存背景文件失败: {error}")
    })?;
    Ok(destination.to_path_buf())
}

fn unique_temp_path(directory: &Path, stem: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    directory.join(format!(".{stem}-{}-{nonce}.tmp", std::process::id()))
}

fn valid_downloaded_asset(path: &Path, extension: &str) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 12];
    if file.read(&mut header).unwrap_or_default() < 12 {
        return false;
    }
    match extension {
        "mp4" => &header[4..8] == b"ftyp",
        "webp" => &header[..4] == b"RIFF" && &header[8..12] == b"WEBP",
        "png" => header.starts_with(b"\x89PNG"),
        "jpg" | "jpeg" => header.starts_with(b"\xff\xd8\xff"),
        "gif" => header.starts_with(b"GIF8"),
        "avif" => &header[4..8] == b"ftyp" && &header[8..12] == b"avif",
        _ => false,
    }
}

fn image_extension(raw_url: &str) -> &'static str {
    let extension = reqwest::Url::parse(raw_url)
        .ok()
        .and_then(|url| {
            Path::new(url.path())
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "png",
        "jpg" => "jpg",
        "jpeg" => "jpeg",
        "gif" => "gif",
        "avif" => "avif",
        _ => "webp",
    }
}

fn discover_endfield_background(
    games_root: &Path,
    hypergryph_app_data: &Path,
) -> Result<Candidate, String> {
    let mut candidates = Vec::new();
    for profile in read_directories(games_root)? {
        let profile_id = profile
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        if profile_id.is_empty() {
            continue;
        }
        let has_endfield_icon = hypergryph_app_data
            .join(&profile_id)
            .join("icon")
            .join("endfield.ico")
            .is_file();
        let leveldb = profile
            .join("cache_storage")
            .join("Local Storage")
            .join("leveldb");
        if !leveldb.is_dir() {
            continue;
        }

        let mut files = read_files(&leveldb)?;
        files.sort_by_key(|path| {
            std::cmp::Reverse(
                path.metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH),
            )
        });
        for path in files {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            for text in decode_leveldb(&bytes) {
                let Some((background, score)) = best_endfield_background(&text) else {
                    continue;
                };
                if !has_endfield_icon && score < 6 {
                    continue;
                }
                candidates.push(Candidate {
                    background,
                    profile_id: profile_id.clone(),
                    profile_root: profile.clone(),
                    source_leveldb: path.clone(),
                    modified,
                    score,
                });
                break;
            }
        }
    }

    candidates.sort_by_key(|candidate| {
        (
            std::cmp::Reverse(candidate.modified),
            std::cmp::Reverse(candidate.score),
        )
    });
    candidates.into_iter().next().ok_or_else(|| {
        "没有找到终末地当前背景。请先打开一次海猫启动器并进入终末地，再让页面完全加载".into()
    })
}

fn read_directories(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| format!("读取启动器数据目录失败: {error}"))?;
    let mut directories = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

fn read_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(root).map_err(|error| format!("读取启动器 LevelDB 失败: {error}"))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_file()).unwrap_or(false) {
            files.push(entry.path());
        }
    }
    Ok(files)
}

fn decode_leveldb(bytes: &[u8]) -> Vec<String> {
    let (utf16, _, _) = encoding_rs::UTF_16LE.decode(bytes);
    let utf8 = String::from_utf8_lossy(bytes);
    if utf16 == utf8 {
        vec![utf16.into_owned()]
    } else {
        vec![utf16.into_owned(), utf8.into_owned()]
    }
}

fn best_endfield_background(text: &str) -> Option<(MainBackground, usize)> {
    let main_pattern = regex::Regex::new(
        r#"(?is)"main_bg_image"\s*:\s*\{(?P<body>[^{}]{0,8192})\}"#,
    )
    .ok()?;
    let url_pattern = regex::Regex::new(r#""url"\s*:\s*"([^"]+)""#).ok()?;
    let md5_pattern = regex::Regex::new(r#""md5"\s*:\s*"([0-9a-fA-F]{32})""#).ok()?;
    let video_pattern = regex::Regex::new(r#""video_url"\s*:\s*"([^"]+)""#).ok()?;

    let captures: Vec<_> = main_pattern.captures_iter(text).collect();
    let mut best = None;
    for (index, capture) in captures.iter().enumerate() {
        let Some(full_match) = capture.get(0) else {
            continue;
        };
        let body = capture.name("body")?.as_str();
        let Some(image_url) = url_pattern
            .captures(body)
            .and_then(|item| item.get(1))
            .map(|item| item.as_str().to_string())
        else {
            continue;
        };
        let Some(image_md5) = md5_pattern
            .captures(body)
            .and_then(|item| item.get(1))
            .map(|item| item.as_str().to_ascii_lowercase())
        else {
            continue;
        };
        if validate_image_url(&image_url).is_err() {
            continue;
        }
        let video_url = video_pattern
            .captures(body)
            .and_then(|item| item.get(1))
            .map(|item| item.as_str().to_string())
            .filter(|url| validate_video_url(url).is_ok());
        let previous_end = index
            .checked_sub(1)
            .and_then(|previous| captures[previous].get(0))
            .map_or(0, |item| item.end());
        let next_start = captures
            .get(index + 1)
            .and_then(|next| next.get(0))
            .map_or(text.len(), |item| item.start());
        let segment_start = previous_end + (full_match.start() - previous_end) / 2;
        let segment_end = full_match.end() + (next_start - full_match.end()) / 2;
        let start = char_boundary_at_or_before(text, segment_start);
        let end = char_boundary_at_or_after(text, segment_end);
        let score = endfield_context_score(&text[start..end]);
        let candidate = (
            MainBackground {
                image_url,
                image_md5,
                video_url,
            },
            score,
        );
        // LevelDB 日志按追加顺序保存记录；最后一个有效 main_bg_image 才是当前背景。
        best = (score > 0).then_some(candidate);
    }
    best
}

fn char_boundary_at_or_before(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn char_boundary_at_or_after(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn endfield_context_score(context: &str) -> usize {
    let text = context.to_ascii_lowercase();
    [
        ("hg_media=endfield", 4),
        ("game.skland.com/endfield", 4),
        ("https://endfield.hypergryph.com", 3),
        ("\"theme\":\"endfield\"", 5),
        ("endfield_incentive", 2),
    ]
    .iter()
    .map(|(marker, weight)| text.matches(marker).count() * weight)
    .sum()
}

fn validate_official_url(raw_url: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw_url).map_err(|error| format!("无效背景 URL: {error}"))?;
    if url.scheme() != "https" {
        return Err("背景 URL 不是 HTTPS".into());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let official = ["hycdn.cn", "hypergryph.com"]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
    if !official {
        return Err(format!("拒绝非鹰角官方背景域名: {host}"));
    }
    Ok(())
}

fn validate_image_url(raw_url: &str) -> Result<(), String> {
    validate_official_url(raw_url)?;
    let extension = reqwest::Url::parse(raw_url)
        .ok()
        .and_then(|url| {
            Path::new(url.path())
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();
    if !["avif", "gif", "jpeg", "jpg", "png", "webp"].contains(&extension.as_str()) {
        return Err(format!("不支持的终末地静态背景格式: {extension}"));
    }
    Ok(())
}

fn validate_video_url(raw_url: &str) -> Result<(), String> {
    validate_official_url(raw_url)?;
    let extension = reqwest::Url::parse(raw_url)
        .ok()
        .and_then(|url| {
            Path::new(url.path())
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();
    if extension != "mp4" {
        return Err(format!("不支持的终末地动态背景格式: {extension}"));
    }
    Ok(())
}

fn find_cached_image(profile_root: &Path, expected_md5: &str) -> Option<PathBuf> {
    let cache = profile_root.join("cache").join("Cache");
    let mut files = read_files(&cache).ok()?;
    files.retain(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with("f_"))
            .unwrap_or(false)
    });
    files.sort_by_key(|path| {
        std::cmp::Reverse(
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        )
    });
    files.into_iter().find(|path| {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        let Some(size) = image_payload_size(&bytes) else {
            return false;
        };
        size <= MAX_IMAGE_BYTES
            && format!("{:x}", md5::compute(&bytes[..size])) == expected_md5.to_ascii_lowercase()
    })
}

fn image_payload_size(bytes: &[u8]) -> Option<usize> {
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let size = 8usize.checked_add(u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize)?;
        return (size <= bytes.len()).then_some(size);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let end = bytes.windows(4).position(|window| window == b"IEND")?;
        return end.checked_add(8).filter(|size| *size <= bytes.len());
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        let end = bytes.windows(2).position(|window| window == b"\xff\xd9")?;
        return end.checked_add(2).filter(|size| *size <= bytes.len());
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return (bytes.len() <= MAX_IMAGE_BYTES).then_some(bytes.len());
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && &bytes[8..12] == b"avif" {
        let size = u32::from_be_bytes(bytes[..4].try_into().ok()?) as usize;
        return (size >= 12 && size <= bytes.len()).then_some(size);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_current_main_background() {
        let text = r#"{
            "main_bg_image": {
                "url": "https://hg-utils-public.hycdn.cn/prod/current.webp",
                "md5": "b406f13a3b71684ebe8c243546b5f2af",
                "video_url": "https://hg-utils-public.hycdn.cn/prod/current.mp4"
            },
            "jump_url": "https://game.skland.com/endfield/game-data",
            "media": "hg_media=endfield"
        }"#;
        let (background, score) = best_endfield_background(text).unwrap();
        assert!(score > 0);
        assert_eq!(background.image_md5, "b406f13a3b71684ebe8c243546b5f2af");
        assert_eq!(
            background.video_url.as_deref(),
            Some("https://hg-utils-public.hycdn.cn/prod/current.mp4")
        );
    }

    #[test]
    fn ignores_background_without_endfield_context() {
        let text = r#"{
            "main_bg_image": {
                "url": "https://hg-utils-public.hycdn.cn/prod/arknights.webp",
                "md5": "d08c05f9f68f7e768c6679c29183932e"
            }
        }"#;
        assert!(best_endfield_background(text).is_none());
    }

    #[test]
    fn newer_non_endfield_background_invalidates_older_record() {
        let text = r#"{
            "main_bg_image": {
                "url": "https://hg-utils-public.hycdn.cn/prod/endfield.webp",
                "md5": "b406f13a3b71684ebe8c243546b5f2af"
            },
            "jump_url": "https://game.skland.com/endfield/game-data",
            "main_bg_image": {
                "url": "https://hg-utils-public.hycdn.cn/prod/arknights.webp",
                "md5": "d08c05f9f68f7e768c6679c29183932e"
            }
        }"#;
        assert!(best_endfield_background(text).is_none());
    }

    #[test]
    fn rejects_non_official_urls() {
        assert!(validate_image_url("https://example.com/a.webp").is_err());
        assert!(validate_image_url("http://hg-utils-public.hycdn.cn/a.webp").is_err());
        assert!(validate_video_url("https://hg-utils-public.hycdn.cn/a.webm").is_err());
        assert!(validate_image_url("https://hg-utils-public.hycdn.cn/a.webp").is_ok());
    }

    #[test]
    fn reads_declared_webp_payload_size() {
        let mut bytes = vec![0u8; 20];
        bytes[..4].copy_from_slice(b"RIFF");
        bytes[4..8].copy_from_slice(&12u32.to_le_bytes());
        bytes[8..12].copy_from_slice(b"WEBP");
        assert_eq!(image_payload_size(&bytes), Some(20));
    }

    #[test]
    fn installed_endfield_discovery_smoke() {
        if std::env::var_os("SSMT_ENDFIELD_SMOKE").is_none() {
            return;
        }
        let local_app_data = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        let roaming_app_data = PathBuf::from(std::env::var_os("APPDATA").unwrap());
        let candidate = discover_endfield_background(
            &local_app_data.join("Games"),
            &roaming_app_data.join("Hypergryph"),
        )
        .unwrap();
        assert!(validate_image_url(&candidate.background.image_url).is_ok());
        assert!(find_cached_image(
            &candidate.profile_root,
            &candidate.background.image_md5
        )
        .is_some());
    }

    #[tokio::test]
    async fn installed_endfield_sync_smoke() {
        if std::env::var_os("SSMT_ENDFIELD_SYNC_SMOKE").is_none() {
            return;
        }
        let local_app_data = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        let roaming_app_data = PathBuf::from(std::env::var_os("APPDATA").unwrap());
        let candidate = discover_endfield_background(
            &local_app_data.join("Games"),
            &roaming_app_data.join("Hypergryph"),
        )
        .unwrap();
        let target = std::env::temp_dir().join(format!(
            "ssmt-endfield-sync-smoke-{}",
            std::process::id()
        ));
        fs::create_dir_all(&target).unwrap();
        let first = sync_candidate(&candidate, &target, None, None, None)
            .await
            .unwrap();
        assert!(first.changed);
        assert!(file_matches_md5(Path::new(&first.image_path), &first.image_md5));
        assert!(first
            .video_path
            .as_deref()
            .is_some_and(|path| file_is_mp4(Path::new(path))));

        let second = sync_candidate(
            &candidate,
            &target,
            Some(&first.image_path),
            first.video_url.as_deref(),
            first.video_path.as_deref(),
        )
        .await
        .unwrap();
        assert!(!second.changed);
        assert_eq!(second.image_path, first.image_path);
        assert_eq!(second.video_path, first.video_path);
        fs::remove_dir_all(target).unwrap();
    }
}
