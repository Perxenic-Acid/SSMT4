use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;

struct StagingFile(PathBuf);

impl Drop for StagingFile {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

fn cache_path(root: &Path, sha256: &str) -> Result<PathBuf, String> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid resource SHA-256".into());
    }
    Ok(root.join(format!("{}.asset", sha256.to_ascii_lowercase())))
}

fn hash_file(path: &Path, limit: u64) -> Result<(u64, String), String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut file, &mut buffer).map_err(|error| error.to_string())?;
        if count == 0 { break }
        total += count as u64;
        if total > limit { return Err("resource exceeds size limit".into()) }
        digest.update(&buffer[..count]);
    }
    Ok((total, format!("{:x}", digest.finalize())))
}

pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    sha256: &str,
    expected_size: u64,
    cache_root: &Path,
) -> Result<PathBuf, String> {
    let parsed = reqwest::Url::parse(url).map_err(|error| error.to_string())?;
    if parsed.scheme() != "https" { return Err("resource URL must use HTTPS".into()) }
    if expected_size == 0 || expected_size > MAX_ASSET_BYTES {
        return Err("resource size is invalid or too large".into());
    }
    let expected = sha256.to_ascii_lowercase();
    let destination = cache_path(cache_root, &expected)?;
    if destination.exists() {
        let (size, actual) = hash_file(&destination, MAX_ASSET_BYTES)?;
        if size == expected_size && actual == expected { return Ok(destination) }
        return Err(format!("cached resource is damaged: {}", destination.display()));
    }
    fs::create_dir_all(cache_root).map_err(|error| error.to_string())?;
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?.as_nanos();
    let staged_path = cache_root.join(format!(".download-{}-{suffix}.tmp", std::process::id()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&staged_path)
        .map_err(|error| error.to_string())?;
    let staged = StagingFile(staged_path);
    let mut response = client.get(parsed).send().await.map_err(|error| error.to_string())?
        .error_for_status().map_err(|error| error.to_string())?;
    if response.url().scheme() != "https" {
        return Err("resource download redirected outside HTTPS".into());
    }
    if response.content_length().is_some_and(|length| length > expected_size) {
        return Err("resource exceeds declared size".into());
    }
    let mut digest = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        total += chunk.len() as u64;
        if total > expected_size { return Err("resource exceeds declared size".into()) }
        file.write_all(&chunk).map_err(|error| error.to_string())?;
        digest.update(&chunk);
    }
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    if total != expected_size || format!("{:x}", digest.finalize()) != expected {
        return Err("resource size or SHA-256 mismatch".into());
    }
    if destination.exists() {
        let (size, actual) = hash_file(&destination, MAX_ASSET_BYTES)?;
        if size == expected_size && actual == expected { return Ok(destination) }
        return Err("another download left a conflicting cached resource".into());
    }
    fs::rename(&staged.0, &destination).map_err(|error| error.to_string())?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_rejects_path_injection() {
        let root = Path::new("cache");
        assert!(cache_path(root, "../other").is_err());
        assert_eq!(cache_path(root, &"A".repeat(64)).unwrap(), root.join(format!("{}.asset", "a".repeat(64))));
    }

    #[tokio::test]
    async fn reuses_only_matching_cached_bytes() {
        let root = std::env::temp_dir().join(format!("ssmt-resource-cache-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let bytes = b"cached asset";
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let path = cache_path(&root, &sha256).unwrap();
        fs::write(&path, bytes).unwrap();
        let client = reqwest::Client::new();
        assert_eq!(download_verified(&client, "https://example.com/unreachable", &sha256,
            bytes.len() as u64, &root).await.unwrap(), path);
        fs::write(&path, b"damaged").unwrap();
        assert!(download_verified(&client, "https://example.com/unreachable", &sha256,
            bytes.len() as u64, &root).await.is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
