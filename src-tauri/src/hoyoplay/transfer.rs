use super::{
    client::{self, Download},
    manifest::{self, Chunk},
    task::Task,
    transaction,
};
use reqwest::blocking::Client;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

// 分块采用压缩数据摘要命名，避免同名块或不同渠道污染续传缓存。
pub fn cache_path(cache: &Path, chunk: &Chunk) -> Result<PathBuf, String> {
    let key = format!(
        "{:x}",
        md5::compute(format!(
            "{}:{}:{}:{}",
            chunk.id, chunk.compressed_md5, chunk.md5, chunk.compressed_size
        ))
    );
    transaction::safe(cache, &key)
}

pub fn decode(bytes: &[u8], chunk: &Chunk, compressed: bool) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 != chunk.compressed_size {
        return Err("分块下载长度不匹配".into());
    }
    if !chunk.compressed_md5.is_empty()
        && format!("{:x}", md5::compute(bytes)) != chunk.compressed_md5.to_lowercase()
    {
        return Err("压缩分块校验失败".into());
    }
    let plain = manifest::decompress(bytes, chunk.size, compressed)?;
    if format!("{:x}", md5::compute(&plain)) != chunk.md5.to_lowercase() {
        return Err("解压分块校验失败".into());
    }
    Ok(plain)
}

pub fn chunk(
    http: &Client,
    source: &Download,
    chunk: &Chunk,
    cache: &Path,
    task: &Task,
) -> Result<Vec<u8>, String> {
    task.check()?;
    let path = cache_path(cache, chunk)?;
    let bucket = md5::compute(chunk.id.as_bytes()).0[0] as usize % task.cache_locks.len();
    let _lock = task.cache_locks[bucket].lock().map_err(|e| e.to_string())?;
    let part = path.with_extension("part");
    transaction::safe(
        cache,
        part.file_name().unwrap().to_str().ok_or("缓存路径无效")?,
    )?;
    if path.exists() {
        let bytes = read_bounded(&path, chunk.compressed_size)?;
        if let Ok(plain) = decode(&bytes, chunk, source.compression == 1) {
            return Ok(plain);
        }
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let url = source.url(&chunk.id)?;
    let mut error = String::new();
    for _ in 0..3 {
        task.check()?;
        match download(http, &url, &part, chunk.compressed_size, task).and_then(
            |bytes| match decode(&bytes, chunk, source.compression == 1) {
                Ok(plain) => Ok(plain),
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    Err(e)
                }
            },
        ) {
            Ok(plain) => {
                fs::rename(&part, &path).map_err(|e| e.to_string())?;
                return Ok(plain);
            }
            Err(e) => error = e,
        }
    }
    Err(format!("资源分块重试失败：{error}"))
}

fn read_bounded(path: &Path, size: u64) -> Result<Vec<u8>, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() != size {
        return Ok(vec![]);
    }
    fs::read(path).map_err(|e| e.to_string())
}

pub(super) fn range_start(
    status: reqwest::StatusCode,
    content_range: Option<&str>,
    offset: u64,
    total: u64,
) -> Result<bool, String> {
    if status == reqwest::StatusCode::OK {
        return Ok(false);
    }
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("下载响应错误：{status}"));
    }
    let expected = format!("bytes {offset}-{}/{total}", total - 1);
    if content_range != Some(expected.as_str()) {
        return Err("续传响应区间与请求不符".into());
    }
    Ok(true)
}

fn download(
    http: &Client,
    url: &str,
    part: &Path,
    size: u64,
    task: &Task,
) -> Result<Vec<u8>, String> {
    client::validate_url(url)?;
    let mut offset = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if offset == size {
        return fs::read(part).map_err(|e| e.to_string());
    }
    if offset > size {
        fs::remove_file(part).map_err(|e| e.to_string())?;
        offset = 0;
    }
    let mut request = http
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let response = request.send().map_err(|e| format!("下载连接失败：{e}"))?;
    append_response(response, part, size, offset, task)
}

pub(super) fn append_response(
    mut response: reqwest::blocking::Response,
    part: &Path,
    size: u64,
    mut offset: u64,
    task: &Task,
) -> Result<Vec<u8>, String> {
    let append = range_start(
        response.status(),
        response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok()),
        offset,
        size,
    )?;
    if !append {
        offset = 0;
    }
    if response
        .content_length()
        .is_some_and(|n| n != size - offset)
    {
        return Err("下载响应长度不符".into());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part)
        .map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 65536];
    loop {
        task.check()?;
        let count = response
            .read(&mut buffer)
            .map_err(|e| format!("下载中断，可重试续传：{e}"))?;
        if count == 0 {
            break;
        }
        if offset + count as u64 > size {
            return Err("下载超出官方声明大小".into());
        }
        file.write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        offset += count as u64;
    }
    file.flush().map_err(|e| e.to_string())?;
    if offset != size {
        return Err("下载未完成，可重试续传".into());
    }
    drop(file);
    fs::read(part).map_err(|e| e.to_string())
}
