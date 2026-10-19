//! SSMT Runtime 的发布发现和本地来源选择，不依赖可选插件，也不加载待检查的 DLL。
use crate::config::path_manager::PathManager;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REPOSITORY: &str = "Perxenic-Acid/SSMT-3DMigoto-Runtime";
const ARCHIVE: &str = "SSMT-Runtime-x64.zip";
const CHECKSUMS: &str = "SHA256SUMS.txt";
const MAX_DLL: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 32 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(12);

// rolling 标签不变，必须按资产 ID 和修改时间发现重发；同一进程不逐次启动访问 GitHub。
static LAST_CHECK: tokio::sync::Mutex<Option<(Instant, Option<String>)>> =
    tokio::sync::Mutex::const_new(None);

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    updated_at: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Receipt {
    asset_id: u64,
    asset_updated_at: String,
    archive_sha256: String,
    dll_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSource {
    source_path: String,
    version: String,
    origin: String,
    warning: Option<String>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn limited_read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("文件超过 Runtime 大小限制".into());
    }
    Ok(bytes)
}

fn validate_x64_dll(bytes: &[u8]) -> Result<(), String> {
    let offset = bytes
        .get(60..64)
        .map(|value| u32::from_le_bytes(value.try_into().unwrap()) as usize)
        .ok_or("Runtime 缺少 PE 文件头")?;
    let header = bytes
        .get(offset..offset.saturating_add(24))
        .ok_or("Runtime PE 文件头越界")?;
    if bytes.get(..2) != Some(b"MZ")
        || &header[..4] != b"PE\0\0"
        || &header[4..6] != b"\x64\x86"
        || u16::from_le_bytes([header[22], header[23]]) & 0x2000 == 0
    {
        return Err("Runtime 不是 Windows x64 DLL".into());
    }
    Ok(())
}

#[cfg(windows)]
fn file_version(path: &Path) -> Result<[u16; 4], String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // 仅读取版本资源，绝不 LoadLibrary；UPX 打包后仍保留同一文件版本。
    unsafe {
        let size = GetFileVersionInfoSizeW(name.as_ptr(), std::ptr::null_mut());
        if size == 0 || size as u64 > MAX_DLL {
            return Err("无法读取 Runtime 文件版本".into());
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(name.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut info = std::ptr::null_mut();
        let mut length = 0;
        if VerQueryValueW(
            data.as_ptr().cast(),
            [b'\\' as u16, 0].as_ptr(),
            &mut info,
            &mut length,
        ) == 0
            || info.is_null()
            || length < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
        {
            return Err("Runtime 没有固定文件版本".into());
        }
        let info = std::ptr::read_unaligned(info.cast::<VS_FIXEDFILEINFO>());
        if info.dwSignature != 0xfeef04bd {
            return Err("Runtime 文件版本无效".into());
        }
        if info.dwFileVersionMS == 0 && info.dwFileVersionLS == 0 {
            return Err("Runtime 发布版本为 0.0.0.0，已拒绝自动替换".into());
        }
        Ok([
            (info.dwFileVersionMS >> 16) as u16,
            info.dwFileVersionMS as u16,
            (info.dwFileVersionLS >> 16) as u16,
            info.dwFileVersionLS as u16,
        ])
    }
}

#[cfg(not(windows))]
fn file_version(_path: &Path) -> Result<[u16; 4], String> {
    Err("Runtime 版本检查仅支持 Windows".into())
}

fn cached_runtime(root: &Path) -> Result<(Receipt, PathBuf), String> {
    let receipt: Receipt =
        serde_json::from_slice(&limited_read(&root.join("current.json"), 16 * 1024)?)
            .map_err(|error| error.to_string())?;
    if !valid_hash(&receipt.dll_sha256) || !valid_hash(&receipt.archive_sha256) {
        return Err("Runtime 缓存索引哈希无效".into());
    }
    let path = root.join(format!("{}.dll", receipt.dll_sha256));
    let bytes = limited_read(&path, MAX_DLL)?;
    if hash(&bytes) != receipt.dll_sha256 {
        return Err("Runtime 缓存 DLL 校验失败".into());
    }
    validate_x64_dll(&bytes)?;
    file_version(&path)?;
    Ok((receipt, path))
}

fn asset<'a>(release: &'a Release, name: &str, limit: u64) -> Result<&'a Asset, String> {
    if release.draft || release.tag_name != "rolling" {
        return Err("SSMT Runtime 发布目录不匹配".into());
    }
    let matches: Vec<_> = release
        .assets
        .iter()
        .filter(|item| item.name == name)
        .collect();
    if matches.len() != 1 {
        return Err(format!("SSMT Runtime 发布资产缺失或重复：{name}"));
    }
    let item = matches[0];
    if item.size == 0
        || item.size > limit
        || item.browser_download_url
            != format!("https://github.com/{REPOSITORY}/releases/download/rolling/{name}")
    {
        return Err(format!("SSMT Runtime 发布来源或大小无效：{name}"));
    }
    Ok(item)
}

async fn fetch_limited(client: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response.url().scheme() != "https" {
        return Err("Runtime 下载重定向到非 HTTPS 地址".into());
    }
    if response.content_length().is_some_and(|size| size > limit) {
        return Err("Runtime 下载超出大小限制".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err("Runtime 下载超出大小限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn checksum(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let matches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let digest = words.next()?;
            if words.next()? != ARCHIVE || words.next().is_some() {
                return None;
            }
            Some(digest)
        })
        .collect();
    if matches.len() != 1 || !valid_hash(matches[0]) {
        return Err("SHA256SUMS 缺少有效的 Runtime 归档校验值".into());
    }
    Ok(matches[0].to_ascii_lowercase())
}

fn extract_dll(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    if zip.len() != 1 {
        return Err("Runtime 归档应仅含 d3d11.dll".into());
    }
    let file = zip.by_index(0).map_err(|error| error.to_string())?;
    if file.name() != "d3d11.dll"
        || file.size() == 0
        || file.size() > MAX_DLL
        || file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
    {
        return Err("Runtime 归档文件名、类型或大小无效".into());
    }
    let expected_size = file.size();
    let mut dll = Vec::new();
    file.take(MAX_DLL + 1)
        .read_to_end(&mut dll)
        .map_err(|error| error.to_string())?;
    if dll.len() as u64 != expected_size {
        return Err("Runtime 归档解压大小不匹配".into());
    }
    validate_x64_dll(&dll)?;
    Ok(dll)
}

fn checked_dll(archive: &Asset, expected: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 != archive.size || hash(bytes) != expected {
        return Err("Runtime 下载大小或 SHA-256 校验失败".into());
    }
    extract_dll(bytes)
}

fn atomic_write(
    path: &Path,
    bytes: &[u8],
    validate: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let staged = path.with_extension(format!("{}-{nonce}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        validate(&staged)?;
        fs::rename(&staged, path).map_err(|error| error.to_string())
    })();
    let _ = fs::remove_file(staged);
    result
}

async fn refresh_cache(root: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("SSMT4-Runtime")
        .connect_timeout(Duration::from_secs(4))
        .timeout(UPDATE_TIMEOUT)
        .https_only(true)
        .build()
        .map_err(|error| error.to_string())?;
    let release: Release = serde_json::from_slice(
        &fetch_limited(
            &client,
            &format!("https://api.github.com/repos/{REPOSITORY}/releases/tags/rolling"),
            1024 * 1024,
        )
        .await?,
    )
    .map_err(|error| error.to_string())?;
    let archive = asset(&release, ARCHIVE, MAX_ARCHIVE)?;
    let sums = asset(&release, CHECKSUMS, 16 * 1024)?;
    if let Ok((previous, _)) = cached_runtime(root) {
        if previous.asset_id == archive.id && previous.asset_updated_at == archive.updated_at {
            return Ok(());
        }
    }
    let sum_bytes = fetch_limited(&client, &sums.browser_download_url, sums.size).await?;
    if sum_bytes.len() as u64 != sums.size {
        return Err("Runtime 校验清单大小不匹配".into());
    }
    let expected = checksum(&sum_bytes)?;
    if archive
        .digest
        .as_ref()
        .is_some_and(|digest| digest != &format!("sha256:{expected}"))
    {
        return Err("Runtime GitHub 资产哈希与 SHA256SUMS 不一致".into());
    }
    let bytes = fetch_limited(&client, &archive.browser_download_url, archive.size).await?;
    let dll = checked_dll(archive, &expected, &bytes)?;
    let receipt = Receipt {
        asset_id: archive.id,
        asset_updated_at: archive.updated_at.clone(),
        archive_sha256: expected,
        dll_sha256: hash(&dll),
    };
    publish_cache(root, &receipt, &dll)
}

fn publish_cache(root: &Path, receipt: &Receipt, dll: &[u8]) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    // DLL 与索引都先写同目录暂存文件。校验失败不会破坏上次已验证的缓存。
    let path = root.join(format!("{}.dll", receipt.dll_sha256));
    atomic_write(&path, dll, |path| file_version(path).map(|_| ()))?;
    if let Ok((_, previous)) = cached_runtime(root) {
        if file_version(&previous)? > file_version(&path)? {
            return Ok(());
        }
    }
    atomic_write(
        &root.join("current.json"),
        &serde_json::to_vec(&receipt).map_err(|error| error.to_string())?,
        |_| Ok(()),
    )
}

fn choose_source(
    bundled: &Path,
    target: &Path,
    cache: &Path,
    warning: Option<String>,
) -> Result<RuntimeSource, String> {
    // 同版本优先本机随包构建；只按数值文件版本选较新项，不能以压缩前后哈希不同认定要降级。
    let mut candidates = vec![(bundled.to_path_buf(), "bundled")];
    if let Ok((_, path)) = cached_runtime(cache) {
        candidates.push((path, "github"));
    }
    candidates.push((target.to_path_buf(), "existing"));
    let mut selected = None;
    for (path, origin) in candidates {
        let Ok(bytes) = limited_read(&path, MAX_DLL) else {
            continue;
        };
        if validate_x64_dll(&bytes).is_err() {
            continue;
        }
        let Ok(version) = file_version(&path) else {
            continue;
        };
        if selected
            .as_ref()
            .is_none_or(|(_, _, current)| version > *current)
        {
            selected = Some((path, origin, version));
        }
    }
    let (path, origin, version) =
        selected.ok_or("没有可用的 SSMT Runtime：随包、缓存与游戏目录均缺失或无效")?;
    let result = RuntimeSource {
        source_path: path.to_string_lossy().into_owned(),
        version: version.map(|item| item.to_string()).join("."),
        origin: origin.into(),
        warning,
    };
    println!(
        "[SSMTRuntime] version={} origin={} source={} target={} warning={:?}",
        result.version,
        result.origin,
        result.source_path,
        target.display(),
        result.warning
    );
    Ok(result)
}

#[tauri::command]
pub async fn resolve_ssmt_runtime_source(
    bundled_path: String,
    target_path: String,
) -> Result<RuntimeSource, String> {
    let cache = PathManager::ssmt_global_config_folder()
        .join("Runtime")
        .join("x64");
    let mut last = LAST_CHECK.lock().await;
    if last
        .as_ref()
        .is_none_or(|(time, _)| time.elapsed() >= CHECK_INTERVAL)
    {
        let warning = match tokio::time::timeout(UPDATE_TIMEOUT, refresh_cache(&cache)).await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("连接 SSMT Runtime 发布超时（12 秒）".into()),
        };
        *last = Some((Instant::now(), warning));
    }
    let warning = last.as_ref().and_then(|(_, warning)| warning.clone());
    drop(last);
    tokio::task::spawn_blocking(move || {
        choose_source(
            Path::new(&bundled_path),
            Path::new(&target_path),
            &cache,
            warning,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
#[path = "ssmt_runtime_tests.rs"]
mod tests;
