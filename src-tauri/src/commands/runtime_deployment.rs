//! Runtime 只在一次注入期间占用工作目录；链接失败才复制，清理时核对文件身份。
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Clone)]
pub(crate) struct Deployment {
    path: PathBuf,
    identity: (u64, u64),
    claimed: bool,
}
fn deployments() -> &'static Mutex<HashMap<PathBuf, Deployment>> {
    static ITEMS: OnceLock<Mutex<HashMap<PathBuf, Deployment>>> = OnceLock::new();
    ITEMS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn key(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("Runtime 缺少父目录")?;
    Ok(fs::canonicalize(parent)
        .map_err(|e| e.to_string())?
        .join(path.file_name().ok_or("Runtime 缺少文件名")?))
}
#[cfg(windows)]
fn identity(path: &Path) -> Result<(u64, u64), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}
#[cfg(not(windows))]
fn identity(path: &Path) -> Result<(u64, u64), String> {
    use std::os::unix::fs::MetadataExt;
    let info = fs::metadata(path).map_err(|e| e.to_string())?;
    Ok((info.dev(), info.ino()))
}
fn symlink(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(source, target)
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(source, target)
    }
}
fn materialize(
    source: &Path,
    target: &Path,
    hardlink: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<&'static str, String> {
    if hardlink(source, target).is_ok() {
        return Ok("hardlink");
    }
    if link(source, target).is_ok() {
        return Ok("symlink");
    }
    fs::copy(source, target).map_err(|e| e.to_string())?;
    Ok("copy")
}
#[tauri::command]
pub fn deploy_runtime_for_launch(
    source_path: String,
    target_path: String,
) -> Result<String, String> {
    let source = fs::canonicalize(source_path).map_err(|e| e.to_string())?;
    let target = key(Path::new(&target_path))?;
    if source == target {
        return Err("Runtime 来源不能是注入目录中的遗留 DLL".into());
    }
    let mut items = deployments().lock().map_err(|e| e.to_string())?;
    if items.get(&target).is_some_and(|item| item.claimed) {
        return Err("该 Runtime 正在等待注入完成".into());
    }
    let staging = target.with_extension(format!("ssmt-link-{}", std::process::id()));
    let _ = fs::remove_file(&staging);
    let result = (|| {
        let method = materialize(
            &source,
            &staging,
            |from, to| fs::hard_link(from, to),
            symlink,
        )?;
        let file_id = identity(&staging)?;
        fs::rename(&staging, &target).map_err(|e| e.to_string())?;
        items.insert(
            target.clone(),
            Deployment {
                path: target.clone(),
                identity: file_id,
                claimed: false,
            },
        );
        Ok(method.to_string())
    })();
    let _ = fs::remove_file(&staging);
    result
}
pub(crate) fn claim(directory: &Path) -> Vec<Deployment> {
    let Ok(directory) = fs::canonicalize(directory) else {
        return vec![];
    };
    let Ok(mut items) = deployments().lock() else {
        return vec![];
    };
    items
        .values_mut()
        .filter(|item| item.path.parent() == Some(directory.as_path()) && !item.claimed)
        .map(|item| {
            item.claimed = true;
            item.clone()
        })
        .collect()
}
pub(crate) fn cleanup(leases: &[Deployment]) {
    let Ok(mut items) = deployments().lock() else {
        return;
    };
    for lease in leases {
        if !items
            .get(&lease.path)
            .is_some_and(|current| current.identity == lease.identity)
        {
            continue;
        }
        if identity(&lease.path).is_ok_and(|current| current == lease.identity) {
            if let Err(error) = fs::remove_file(&lease.path) {
                eprintln!(
                    "[RuntimeDeployment] 撤除 {} 失败：{error}",
                    lease.path.display()
                );
                if let Some(item) = items.get_mut(&lease.path) {
                    item.claimed = false;
                }
                continue;
            }
        }
        items.remove(&lease.path);
    }
}
#[tauri::command]
pub fn cancel_runtime_deployment(directory: String) {
    let leases = claim(Path::new(&directory));
    cleanup(&leases);
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "ssmt-deployment-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn denied(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::ErrorKind::PermissionDenied.into())
    }
    #[test]
    fn hardlink_deployment_is_removed_without_deleting_source() {
        let fixture = Fixture::new();
        let source = fixture.0.join("resource.dll");
        let target = fixture.0.join("d3d11.dll");
        fs::write(&source, b"runtime").unwrap();
        fs::write(&target, b"old").unwrap();
        assert_eq!(
            deploy_runtime_for_launch(
                source.to_string_lossy().into(),
                target.to_string_lossy().into()
            )
            .unwrap(),
            "hardlink"
        );
        assert_eq!(identity(&source).unwrap(), identity(&target).unwrap());
        let leases = claim(&fixture.0);
        assert_eq!(leases.len(), 1);
        assert!(deploy_runtime_for_launch(
            source.to_string_lossy().into(),
            target.to_string_lossy().into()
        )
        .is_err());
        cleanup(&leases);
        assert!(!target.exists());
        assert_eq!(fs::read(source).unwrap(), b"runtime");
    }
    #[test]
    fn fallback_order_and_copy_contents() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let target = fixture.0.join("target");
        fs::write(&source, b"runtime").unwrap();
        assert_eq!(
            materialize(&source, &target, denied, |from, to| fs::hard_link(from, to)).unwrap(),
            "symlink"
        );
        fs::remove_file(&target).unwrap();
        assert_eq!(
            materialize(&source, &target, denied, denied).unwrap(),
            "copy"
        );
        assert_ne!(identity(&source).unwrap(), identity(&target).unwrap());
        assert_eq!(fs::read(target).unwrap(), b"runtime");
    }
    #[test]
    fn cleanup_preserves_a_file_replaced_after_deployment() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let target = fixture.0.join("d3d11.dll");
        fs::write(&source, b"runtime").unwrap();
        deploy_runtime_for_launch(
            source.to_string_lossy().into(),
            target.to_string_lossy().into(),
        )
        .unwrap();
        let leases = claim(&fixture.0);
        fs::remove_file(&target).unwrap();
        fs::write(&target, b"user replacement").unwrap();
        cleanup(&leases);
        assert_eq!(fs::read(target).unwrap(), b"user replacement");
    }

    #[cfg(windows)]
    #[test]
    fn loaded_module_can_keep_running_after_its_deployment_is_unlinked() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::FreeLibrary;
        use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
        let fixture = Fixture::new();
        let source = fixture.0.join("resource.dll");
        let target = fixture.0.join("d3d11.dll");
        let system =
            PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/version.dll");
        fs::copy(system, &source).unwrap();
        deploy_runtime_for_launch(
            source.to_string_lossy().into(),
            target.to_string_lossy().into(),
        )
        .unwrap();
        let leases = claim(&fixture.0);
        let name: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        assert!(!module.is_null());
        cleanup(&leases);
        let removed = !target.exists();
        let source_kept = source.exists();
        assert_ne!(unsafe { FreeLibrary(module) }, 0);
        assert!(removed);
        assert!(source_kept);
    }
}
