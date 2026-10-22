use super::{client, engine::Request, manifest, transaction, Context};
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub const WORKERS: u64 = 4;
pub const MAX_CHUNK: u64 = 64 * 1024 * 1024;
pub const SHARED_CACHE: u64 = 256 * 1024 * 1024;
// 包括小文件、文件系统分配取整，以及清单之外的渠道 SDK（压缩 128 MiB、解压 512 MiB）。
pub const RESERVE: u64 = 768 * 1024 * 1024;

pub fn sum(values: impl IntoIterator<Item = u64>) -> Result<u64, String> {
    values.into_iter().try_fold(0u64, |a, b| {
        a.checked_add(b).ok_or_else(|| "磁盘需求溢出".into())
    })
}

pub fn streaming_required(staged: u64) -> Result<u64, String> {
    sum([staged, WORKERS * MAX_CHUNK, SHARED_CACHE, RESERVE])
}

pub fn staged_credit(path: &Path, size: u64) -> Result<u64, String> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("无法读取暂存文件大小：{e}")),
    };
    if !meta.is_file() {
        return Err("暂存文件位置被目录占用".into());
    }
    // 稀疏 / 文件系统压缩不能用逻辑长度抵扣真实磁盘分配。
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & (0x200 | 0x800) != 0 {
            return Ok(0);
        }
    }
    Ok(meta.len().min(size))
}

fn existing_work(root: &Path) -> Option<PathBuf> {
    let root = fs::canonicalize(root).ok()?;
    let work = root.join(".ssmt-maintenance");
    let meta = fs::symlink_metadata(&work).ok()?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return None;
        }
    }
    let owner =
        transaction::read_limited(&transaction::safe(&work, "owner.json").ok()?, 8192).ok()?;
    let owner: serde_json::Value = serde_json::from_slice(&owner).ok()?;
    (owner == serde_json::json!({"schema":1,"owner":"SSMT.HoYoPlay","root":root})).then_some(work)
}

fn remaining_game_bytes(
    manifests: &[client::ManifestInfo],
    work: Option<&Path>,
) -> Result<u64, String> {
    let mut sizes = vec![];
    for info in manifests {
        let mut remaining = info.install_size;
        if let Some(work) = work {
            let directory = transaction::safe(work, "manifests")?;
            // 续装只读取已经校验过的缓存清单；首次预估不下载大清单，也不修改目录。
            if let Some(files) = manifest::read_cached(info, &directory)? {
                for asset in files.iter().filter(|f| !f.folder) {
                    let stage = transaction::safe(work, &format!("stage/{}", asset.name))?;
                    remaining = remaining
                        .checked_sub(staged_credit(&stage, asset.size)?)
                        .ok_or("官方资源统计与清单大小不一致")?;
                }
            }
        }
        sizes.push(remaining);
    }
    sum(sizes)
}

pub fn available(root: &Path) -> Result<u64, String> {
    if !root.is_absolute()
        || root.parent().is_none()
        || root.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("请选择明确的游戏目录，不能使用磁盘根目录或相对路径".into());
    }
    // 允许预览尚未创建的安装目录；只查询其所在卷，不创建目录或事务。
    let mut existing = root;
    while !existing.exists() {
        existing = existing.parent().ok_or("无法找到安装目录所在磁盘")?;
    }
    if !existing.is_dir() {
        return Err("安装位置不是文件夹".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut bytes = 0;
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut bytes,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "无法读取磁盘剩余空间：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(bytes)
    }
    #[cfg(not(windows))]
    {
        Err("当前平台暂不支持安装磁盘空间查询".into())
    }
}

pub fn check(root: &Path, needed: u64) -> Result<(), String> {
    let free = available(root)?;
    if free < needed {
        return Err(format!(
            "磁盘空间不足：本次还需约 {:.2} GiB，可用 {:.2} GiB",
            needed as f64 / 1073741824.,
            free as f64 / 1073741824.
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    target_version: String,
    download_bytes: u64,
    game_bytes: u64,
    required_bytes: u64,
    available_bytes: u64,
    sufficient: bool,
}

pub fn estimate(request: Request) -> Result<Estimate, String> {
    if request.operation != "install" {
        return Err("空间预估目前仅用于首次安装".into());
    }
    if request.audio_languages.is_empty()
        || request
            .audio_languages
            .iter()
            .any(|v| !client::AUDIO.contains(&v.as_str()))
    {
        return Err("请至少选择一种受支持的语音".into());
    }
    let root = Path::new(&request.directory);
    available(root)?;
    let context = Context::load(&request.game_name)?;
    let servers = client::servers(&context.preset);
    let server = servers
        .iter()
        .find(|s| s.id == request.server_id)
        .ok_or("未知游戏渠道")?;
    let http = client::http()?;
    let (branch, _) = client::branches(&http, server)?;
    let manifests = client::build(&http, server, &branch, &request.audio_languages)?;
    let game_bytes = sum(manifests.iter().map(|m| m.install_size))?;
    // Build 的统计可能重复计入跨类别共享文件；保守展示上限，不为此下载完整游戏清单。
    let download_bytes = sum(manifests.iter().map(|m| m.download_size))?;
    let remaining = remaining_game_bytes(&manifests, existing_work(root).as_deref())?;
    let required_bytes = sum([
        streaming_required(remaining)?,
        sum(manifests.iter().map(|m| m.size))?,
    ])?;
    let free = available(root)?;
    Ok(Estimate {
        target_version: branch.tag,
        download_bytes,
        game_bytes,
        required_bytes,
        available_bytes: free,
        sufficient: free >= required_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    #[test]
    fn streaming_budget_does_not_add_a_full_download_and_checks_overflow() {
        let game = 100 * 1024 * 1024 * 1024;
        assert_eq!(streaming_required(game).unwrap(), game + 1280 * 1024 * 1024);
        assert!(streaming_required(u64::MAX).is_err());
        assert!(sum([u64::MAX, 1]).is_err());
    }
    #[test]
    fn preview_queries_parent_without_creating_installation_directory() {
        let temp = super::super::tests::Temp::new();
        let target = temp.0.join("not-created/game");
        assert!(available(&target).unwrap() > 0);
        assert!(!target.exists());
        assert!(available(Path::new("relative/game")).is_err());
        let traversal = PathBuf::from(format!("{}\\..\\outside", temp.0.display()));
        assert!(available(&traversal).is_err());
        std::fs::write(temp.0.join("file"), b"x").unwrap();
        assert!(available(&temp.0.join("file/game")).is_err());
    }

    #[test]
    fn resume_estimate_credits_only_owned_stage_files_from_verified_matching_manifests() {
        let temp = super::super::tests::Temp::new();
        let work = transaction::Workspace::open(&temp.0.join("game")).unwrap();
        let asset = manifest::Asset {
            name: "Game_Data/file".into(),
            size: 8,
            folder: false,
            md5: format!("{:x}", md5::compute(b"ABCDEFGH")),
            chunks: vec![manifest::Chunk {
                id: "fixture".into(),
                size: 8,
                offset: 0,
                compressed_size: 8,
                md5: format!("{:x}", md5::compute(b"ABCDEFGH")),
                compressed_md5: String::new(),
            }],
        };
        let bytes = manifest::Manifest { files: vec![asset] }.encode_to_vec();
        let info = client::ManifestInfo {
            compressed: false,
            checksum: format!("{:x}", md5::compute(&bytes)),
            size: bytes.len() as u64,
            unpacked_size: bytes.len() as u64,
            url: "unsupported://must-not-download".into(),
            chunks: client::Download {
                url_prefix: String::new(),
                url_suffix: String::new(),
                compression: 0,
                encryption: 0,
            },
            matching_field: "game".into(),
            download_size: 8,
            install_size: 8,
        };
        fs::create_dir_all(work.stage.join("Game_Data")).unwrap();
        fs::write(work.stage.join("Game_Data/file"), b"ABCD").unwrap();
        fs::write(work.stage.join("irrelevant"), vec![0; 100]).unwrap();
        let manifests = work.work.join("manifests");
        fs::create_dir(&manifests).unwrap();
        let cached = manifests.join(format!("{}-{}.manifest", info.checksum, info.size));
        assert_eq!(
            remaining_game_bytes(&[info.clone()], Some(&work.work)).unwrap(),
            8
        );
        fs::write(&cached, &bytes).unwrap();
        assert_eq!(existing_work(&work.root).unwrap(), work.work);
        assert_eq!(
            remaining_game_bytes(&[info.clone()], Some(&work.work)).unwrap(),
            4
        );
        fs::write(&cached, b"corrupt").unwrap();
        assert_eq!(remaining_game_bytes(&[info], Some(&work.work)).unwrap(), 8);
        fs::write(work.work.join("owner.json"), b"{}").unwrap();
        assert!(existing_work(&work.root).is_none());
    }
}
