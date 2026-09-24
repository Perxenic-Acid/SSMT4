use super::{LockOwner, LockReport};
use std::{collections::BTreeSet, ffi::c_void, os::windows::ffi::OsStrExt, path::PathBuf};
type Handle = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct UniqueProcess {
    pid: u32,
    start: FileTime,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessInfo {
    process: UniqueProcess,
    name: [u16; 256],
    service: [u16; 64],
    app_type: u32,
    status: u32,
    session: u32,
    restartable: i32,
}
#[link(name = "Rstrtmgr")]
extern "system" {
    fn RmStartSession(session: *mut u32, flags: u32, key: *mut u16) -> u32;
    fn RmEndSession(session: u32) -> u32;
    fn RmRegisterResources(
        session: u32,
        files: u32,
        names: *const *const u16,
        apps: u32,
        processes: *const UniqueProcess,
        services: u32,
        service_names: *const *const u16,
    ) -> u32;
    fn RmGetList(
        session: u32,
        needed: *mut u32,
        count: *mut u32,
        info: *mut ProcessInfo,
        reasons: *mut u32,
    ) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    #[cfg(test)]
    fn LoadLibraryW(path: *const u16) -> Handle;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetProcessTimes(
        handle: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn QueryFullProcessImageNameW(
        handle: Handle,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn IsProcessCritical(handle: Handle, critical: *mut i32) -> i32;
    fn TerminateProcess(handle: Handle, code: u32) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
}
#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationFile(
        handle: Handle,
        status: *mut usize,
        info: *mut c_void,
        length: u32,
        class: u32,
    ) -> i32;
}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Session(u32);
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            RmEndSession(self.0);
        }
    }
}
fn text(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(wide.len())])
}
fn start_time(handle: Handle) -> Option<String> {
    let (mut creation, mut exit, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some(format!(
        "{}",
        ((creation.high as u64) << 32) | creation.low as u64
    ))
}
fn owner(pid: u32) -> LockOwner {
    let mut result = LockOwner {
        pid,
        name: format!("PID {pid}"),
        executable: String::new(),
        started: String::new(),
        can_terminate: false,
    };
    let raw = unsafe { OpenProcess(0x1000, 0, pid) }; // QUERY_LIMITED_INFORMATION
    if raw.is_null() {
        return result;
    }
    let handle = OwnedHandle(raw);
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, name.as_mut_ptr(), &mut size) } != 0 {
        result.executable = String::from_utf16_lossy(&name[..size as usize]);
        result.name = result
            .executable
            .rsplit('\\')
            .next()
            .unwrap_or(&result.executable)
            .to_owned();
    }
    result.started = start_time(handle.0).unwrap_or_default();
    let mut critical = 1;
    let checked = unsafe { IsProcessCritical(handle.0, &mut critical) } != 0;
    result.can_terminate = checked
        && critical == 0
        && pid > 4
        && pid != std::process::id()
        && !result.started.is_empty()
        && !result.executable.is_empty();
    result
}

// Restart Manager accepts files, not directories. Query directory handles separately.
// FILE_PROCESS_IDS_USING_FILE_INFORMATION is ULONG followed by aligned ULONG_PTR[].
fn directory_owners(path: &PathBuf) -> Result<Vec<u32>, String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0x80,
            7,
            std::ptr::null_mut(),
            3,
            0x02000000,
            std::ptr::null_mut(),
        )
    };
    if raw as isize == -1 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let handle = OwnedHandle(raw);
    let mut buffer = vec![0usize; 8192];
    let mut io_status = [0usize; 2];
    let status = unsafe {
        NtQueryInformationFile(
            handle.0,
            io_status.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            (buffer.len() * std::mem::size_of::<usize>()) as u32,
            47,
        )
    };
    if status < 0 {
        return Err(format!("目录句柄查询失败 (NTSTATUS {status:#x})"));
    }
    let count = (buffer[0] & 0xffff_ffff) as usize;
    if count >= buffer.len() {
        return Err("目录句柄列表过大".into());
    }
    Ok(buffer[1..1 + count]
        .iter()
        .filter_map(|pid| u32::try_from(*pid).ok())
        .filter(|pid| *pid != std::process::id())
        .collect())
}

pub fn scan(paths: &[PathBuf], report: &mut LockReport) -> Result<(), String> {
    let mut session = 0;
    let mut key = [0u16; 33];
    let status = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    if status != 0 {
        return Err(format!("无法启动 Windows 占用诊断：{status}"));
    }
    let session = Session(session);
    let mut pids = BTreeSet::new();
    let mut directory_failures = 0;
    let files: Vec<Vec<u16>> = paths
        .iter()
        .filter_map(|p| {
            if p.is_dir() {
                match directory_owners(p) {
                    Ok(found) => pids.extend(found),
                    Err(_) => directory_failures += 1,
                }
                None
            } else {
                Some(p.as_os_str().encode_wide().chain(Some(0)).collect())
            }
        })
        .collect();
    if directory_failures > 0 {
        report.warnings.push(format!(
            "{directory_failures} 个目录无法查询句柄，结果可能不完整。"
        ));
    }
    for batch in files.chunks(256) {
        let names: Vec<*const u16> = batch.iter().map(|s| s.as_ptr()).collect();
        let status = unsafe {
            RmRegisterResources(
                session.0,
                names.len() as u32,
                names.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if status != 0 {
            return Err(format!("无法注册待检查文件：Windows {status}"));
        }
    }
    if !files.is_empty() {
        let mut needed = 0;
        let mut count = 0;
        let mut reasons = 0;
        let mut status = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut count,
                std::ptr::null_mut(),
                &mut reasons,
            )
        };
        for _ in 0..8 {
            if status != 234 {
                break;
            } // ERROR_MORE_DATA; list may change while querying.
            let mut entries: Vec<ProcessInfo> =
                vec![unsafe { std::mem::zeroed() }; needed as usize];
            count = needed;
            status = unsafe {
                RmGetList(
                    session.0,
                    &mut needed,
                    &mut count,
                    entries.as_mut_ptr(),
                    &mut reasons,
                )
            };
            if status == 0 {
                for entry in entries.iter().take(count as usize) {
                    let mut process = owner(entry.process.pid);
                    let rm_start = format!(
                        "{}",
                        ((entry.process.start.high as u64) << 32) | entry.process.start.low as u64
                    );
                    if process.started != rm_start {
                        process.can_terminate = false;
                    }
                    if process.executable.is_empty() {
                        process.name = text(&entry.name);
                    }
                    pids.remove(&process.pid);
                    report.owners.push(process);
                }
            }
        }
        if status != 0 {
            report.warnings.push(format!(
                "Windows 文件占用查询不完整：{status}。可尝试以管理员身份重试。"
            ));
        }
    }
    report.owners.extend(pids.into_iter().map(owner));
    Ok(())
}

pub fn terminate(process: &LockOwner) -> Result<(), String> {
    if !process.can_terminate || process.pid <= 4 || process.pid == std::process::id() {
        return Err("不能结束此进程".into());
    }
    let raw = unsafe { OpenProcess(0x1000 | 0x100000 | 1, 0, process.pid) };
    if raw.is_null() {
        return Err(format!(
            "无法结束 {}：{}",
            process.name,
            std::io::Error::last_os_error()
        ));
    }
    let handle = OwnedHandle(raw);
    if start_time(handle.0).as_deref() != Some(process.started.as_str()) {
        return Err("进程已变化，请重新查询占用".into());
    }
    let mut critical = 1;
    if unsafe { IsProcessCritical(handle.0, &mut critical) } == 0 || critical != 0 {
        return Err("不能结束系统关键进程".into());
    }
    if unsafe { TerminateProcess(handle.0, 1) } == 0 {
        return Err(format!(
            "结束 {} 失败：{}",
            process.name,
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { WaitForSingleObject(handle.0, 5000) } != 0 {
        return Err(format!("等待 {} 退出超时，请重新查询", process.name));
    }
    // Explorer provides the desktop shell. Restore it after the explicitly approved exit.
    if process.name.eq_ignore_ascii_case("explorer.exe") {
        let shell = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .ok_or("无法定位 Windows 目录，请手动运行 explorer.exe")?
            .join("explorer.exe");
        std::process::Command::new(shell)
            .spawn()
            .map_err(|e| format!("资源管理器已结束，但重启失败，请手动运行 explorer.exe：{e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Child, Command},
        time::{Duration, Instant},
    };

    struct ChildLock {
        child: Child,
        root: PathBuf,
    }
    impl Drop for ChildLock {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn child_lock(kind: &str) -> ChildLock {
        let root =
            std::env::temp_dir().join(format!("ssmt-lock-test-{}-{kind}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("locked.dat"), b"test").unwrap();
        let helper = format!(
            "{}::lock_child",
            std::thread::current()
                .name()
                .unwrap()
                .rsplit_once("::")
                .unwrap()
                .0
        );
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", &helper])
            .env("SSMT_LOCK_TEST_ROOT", &root)
            .env("SSMT_LOCK_TEST_KIND", kind)
            .spawn()
            .unwrap();
        let mut guard = ChildLock { child, root };
        let started = Instant::now();
        while !guard.root.join("ready").exists() {
            assert!(
                guard.child.try_wait().unwrap().is_none(),
                "lock child exited"
            );
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "lock child timeout"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
        guard
    }
    #[test]
    #[ignore = "helper subprocess"]
    fn lock_child() {
        let root = PathBuf::from(std::env::var_os("SSMT_LOCK_TEST_ROOT").unwrap());
        let kind = std::env::var("SSMT_LOCK_TEST_KIND").unwrap();
        if kind == "dll" {
            let library = root.join("d3d11.dll");
            let windows = PathBuf::from(std::env::var_os("WINDIR").unwrap());
            std::fs::copy(windows.join("System32/version.dll"), &library).unwrap();
            let wide: Vec<u16> = library.as_os_str().encode_wide().chain(Some(0)).collect();
            assert!(!unsafe { LoadLibraryW(wide.as_ptr()) }.is_null());
            std::fs::write(root.join("ready"), b"ready").unwrap();
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let path = if kind == "directory" {
            root.clone()
        } else {
            root.join("locked.dat")
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0x80000000,
                1,
                std::ptr::null_mut(),
                3,
                0x02000000,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(raw as isize, -1, "{}", std::io::Error::last_os_error());
        let _handle = OwnedHandle(raw);
        std::fs::write(root.join("ready"), b"ready").unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }
    #[test]
    fn finds_file_owner_and_rejects_reused_identity() {
        let guard = child_lock("file");
        let mut report = LockReport::default();
        scan(&[guard.root.join("locked.dat")], &mut report).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let found = report
            .owners
            .iter()
            .find(|p| p.pid == guard.child.id())
            .expect("file owner");
        assert!(found.can_terminate);
        let renamed = guard.root.with_extension("renamed");
        assert!(std::fs::rename(&guard.root, &renamed).is_err());
        let mut stale = found.clone();
        stale.started = "0".into();
        assert!(terminate(&stale).is_err());
        terminate(found).unwrap();
        std::fs::rename(&guard.root, &renamed).unwrap();
        std::fs::rename(&renamed, &guard.root).unwrap();
    }
    #[test]
    fn finds_loaded_dll_by_full_path() {
        let guard = child_lock("dll");
        let mut report = LockReport::default();
        scan(&[guard.root.join("d3d11.dll")], &mut report).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(
            report.owners.iter().any(|p| p.pid == guard.child.id()),
            "loaded DLL owner not found"
        );
        let other = guard.root.join("other");
        std::fs::create_dir(&other).unwrap();
        std::fs::copy(guard.root.join("d3d11.dll"), other.join("d3d11.dll")).unwrap();
        let mut unrelated = LockReport::default();
        scan(&[other.join("d3d11.dll")], &mut unrelated).unwrap();
        assert!(!unrelated.owners.iter().any(|p| p.pid == guard.child.id()));
    }
    #[test]
    fn finds_directory_owner() {
        let guard = child_lock("directory");
        assert!(directory_owners(&guard.root)
            .unwrap()
            .contains(&guard.child.id()));
    }
    #[test]
    fn empty_directory_query_and_self_protection() {
        let root = std::env::temp_dir().join(format!("ssmt-lock-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(directory_owners(&root).unwrap().is_empty());
        assert!(!owner(std::process::id()).can_terminate);
        std::fs::remove_dir(root).unwrap();
    }
}
