use super::client::{self, Client, ManifestInfo};
use prost::Message;
use std::{collections::HashSet, io::Read, path::PathBuf};

#[derive(Clone, PartialEq, Message)]
pub struct Manifest {
    #[prost(message, repeated, tag = "1")]
    pub files: Vec<Asset>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Asset {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(message, repeated, tag = "2")]
    pub chunks: Vec<Chunk>,
    #[prost(bool, tag = "3")]
    pub folder: bool,
    #[prost(uint64, tag = "4")]
    pub size: u64,
    #[prost(string, tag = "5")]
    pub md5: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct Chunk {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub md5: String,
    #[prost(uint64, tag = "3")]
    pub offset: u64,
    #[prost(uint64, tag = "4")]
    pub compressed_size: u64,
    #[prost(uint64, tag = "5")]
    pub size: u64,
    #[prost(string, tag = "7")]
    pub compressed_md5: String,
}

pub fn relative(raw: &str) -> Result<PathBuf, String> {
    let value = raw.replace('\\', "/");
    if value.is_empty() || value.starts_with('/') || value.contains(':') || value.contains('\0') {
        return Err(format!("非法游戏资源路径：{raw}"));
    }
    for component in value.split('/') {
        let base = component
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.chars().any(|c| c < ' ' || "<>\"|?*".contains(c))
            || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.as_bytes()[3].is_ascii_digit())
        {
            return Err(format!("非法游戏资源路径：{raw}"));
        }
    }
    if value
        .split('/')
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case(".ssmt-maintenance"))
    {
        return Err("游戏资源与 SSMT 事务目录冲突".into());
    }
    Ok(PathBuf::from(value))
}

pub fn decompress(bytes: &[u8], size: u64, compressed: bool) -> Result<Vec<u8>, String> {
    if size > 128 * 1024 * 1024 {
        return Err("资源解压大小超出限制".into());
    }
    let result = if compressed {
        let decoder = zstd::stream::read::Decoder::new(bytes).map_err(|e| e.to_string())?;
        let mut result = vec![];
        decoder
            .take(size + 1)
            .read_to_end(&mut result)
            .map_err(|e| format!("Zstd 解压失败：{e}"))?;
        result
    } else {
        bytes.to_vec()
    };
    if result.len() as u64 != size {
        return Err("解压后的资源大小不匹配".into());
    }
    Ok(result)
}

#[cfg(test)]
pub fn decode(client: &Client, info: &ManifestInfo) -> Result<Vec<Asset>, String> {
    if info.size > 64 * 1024 * 1024 {
        return Err("清单压缩大小超出限制".into());
    }
    let bytes = client::get_bytes(client, &info.url, info.size)?;
    decode_bytes(&bytes, info)
}

pub fn cached(
    client: &Client,
    info: &ManifestInfo,
    directory: &std::path::Path,
) -> Result<Vec<Asset>, String> {
    if let Some(files) = read_cached(info, directory)? {
        return Ok(files);
    }
    let bytes = client::get_bytes(client, &info.url, info.size)?;
    let files = decode_bytes(&bytes, info)?;
    std::fs::write(cache_path(info, directory)?, bytes).map_err(|e| e.to_string())?;
    Ok(files)
}

fn cache_path(info: &ManifestInfo, directory: &std::path::Path) -> Result<PathBuf, String> {
    client::validate_md5(&info.checksum)?;
    if info.size > 64 * 1024 * 1024 {
        return Err("清单压缩大小超出限制".into());
    }
    super::transaction::safe(
        directory,
        &format!("{}-{}.manifest", info.checksum, info.size),
    )
}

pub fn read_cached(
    info: &ManifestInfo,
    directory: &std::path::Path,
) -> Result<Option<Vec<Asset>>, String> {
    let path = cache_path(info, directory)?;
    if path.is_file() {
        if let Ok(cached) = super::transaction::read_limited(&path, info.size) {
            if let Ok(files) = decode_bytes(&cached, info) {
                return Ok(Some(files));
            }
        }
    }
    Ok(None)
}

fn decode_bytes(bytes: &[u8], info: &ManifestInfo) -> Result<Vec<Asset>, String> {
    if bytes.len() as u64 != info.size {
        return Err("官方清单下载不完整".into());
    }
    let plain = decompress(bytes, info.unpacked_size, info.compressed)?;
    if format!("{:x}", md5::compute(&plain)) != info.checksum {
        return Err("官方清单 MD5 不匹配".into());
    }
    let manifest =
        Manifest::decode(plain.as_slice()).map_err(|e| format!("Sophon 清单无法解析：{e}"))?;
    validate(&manifest.files)?;
    Ok(manifest.files)
}

pub fn validate(files: &[Asset]) -> Result<(), String> {
    let mut paths = HashSet::new();
    if files.len() > 1_000_000 {
        return Err("清单文件数超出限制".into());
    }
    for file in files {
        relative(&file.name)?;
        if !paths.insert(file.name.replace('\\', "/").to_lowercase()) {
            return Err("清单含大小写冲突或重复路径".into());
        }
        if file.folder {
            if file.size != 0 || !file.chunks.is_empty() {
                return Err("清单目录含文件数据".into());
            }
            continue;
        }
        client::validate_md5(&file.md5)?;
        let mut end = 0;
        for chunk in &file.chunks {
            client::validate_md5(&chunk.md5)?;
            if !chunk.compressed_md5.is_empty() {
                client::validate_md5(&chunk.compressed_md5)?;
            }
            if chunk.id.is_empty()
                || !chunk
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
                || chunk.size == 0
                || chunk.size > 64 * 1024 * 1024
                || chunk.compressed_size > 64 * 1024 * 1024
                || chunk.compressed_size == 0
                || chunk.offset != end
            {
                return Err("Sophon 分块标识、大小或布局无效".into());
            }
            end = end.checked_add(chunk.size).ok_or("资源大小溢出")?;
        }
        if end != file.size {
            return Err(format!("文件分块长度不匹配：{}", file.name));
        }
    }
    let file_paths: HashSet<_> = files
        .iter()
        .filter(|f| !f.folder)
        .map(|f| f.name.replace('\\', "/").to_lowercase())
        .collect();
    for path in &paths {
        let mut parent = path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if file_paths.contains(prefix) {
                return Err("清单同时把同一路径用于文件和父目录".into());
            }
            parent = prefix;
        }
    }
    Ok(())
}
