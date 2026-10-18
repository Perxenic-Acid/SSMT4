//! 可选启动计划的只读日志来源。只在目标 PID 已确认后跟随，不接管 DLL 的标准输入。
use super::launch_console::{ConsoleEvent, ConsoleLog, StreamText, log_output, nonce};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime},
};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchLogSource {
    pub name: String,
    pub path: PathBuf,
}

pub struct LogTail {
    source: LaunchLogSource,
    offset: u64,
    created: Option<SystemTime>,
    prefix: Vec<u8>,
    decoder: StreamText,
}

impl LogTail {
    pub fn new(source: LaunchLogSource) -> Self {
        let mut tail = Self {
            source,
            offset: 0,
            created: None,
            prefix: Vec::new(),
            decoder: StreamText::new(),
        };
        // 基线在提交启动前建立；不把上次运行的内容当成本次输出。
        if let Ok(mut file) = tail.open() {
            if let Ok(metadata) = file.metadata() {
                tail.offset = metadata.len();
                tail.created = metadata.created().ok();
                let _ = (&mut file).take(128).read_to_end(&mut tail.prefix);
            }
        }
        tail
    }

    fn open(&self) -> std::io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // ReShade 可重写、轮转和删除日志；观察者不能锁住它。
            options.share_mode(1 | 2 | 4);
        }
        options.open(&self.source.path)
    }

    pub fn read(&mut self) -> std::io::Result<String> {
        let mut file = match self.open() {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(e) => return Err(e),
        };
        let metadata = file.metadata()?;
        let created = metadata.created().ok();
        let mut prefix = Vec::new();
        (&mut file).take(128).read_to_end(&mut prefix)?;
        if metadata.len() < self.offset
            || created != self.created
            || !prefix.starts_with(&self.prefix)
        {
            self.offset = 0;
            self.decoder = StreamText::new();
        }
        self.created = created;
        self.prefix = prefix;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        (&mut file).take(32 * 1024).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        Ok(self.decoder.decode(&bytes, false))
    }
}

static FOLLOWERS: AtomicUsize = AtomicUsize::new(0);

pub fn follow(app: AppHandle, sources: Vec<LogTail>, pid: u32, parent: &str) {
    for mut tail in sources {
        if FOLLOWERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 16).then_some(n + 1)
            })
            .is_err()
        {
            break;
        }
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                FOLLOWERS.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let guard = Guard;
        let app = app.clone();
        let parent = parent.to_owned();
        std::thread::spawn(move || {
            let _guard = guard;
            let id = nonce();
            let program = tail.source.name.clone();
            let publish = |kind: &str, text: String| {
                let _ = app.emit(
                    "launch-console-event",
                    ConsoleEvent {
                        session_id: id.clone(),
                        program: program.clone(),
                        kind: kind.into(),
                        text,
                        pid: Some(pid),
                        code: None,
                    },
                );
            };
            // 单独的事件种类表示只读来源；前端不能向其发送输入。
            publish("log_started", parent);
            let mut log = ConsoleLog::new(
                &crate::config::path_manager::PathManager::ssmt_cache_root(&app).join("Logs"),
                &id,
            )
            .ok();
            log_output(&mut log, "source", &tail.source.path.to_string_lossy());
            if let Some(writer) = log.as_mut() {
                let _ = writer.flush();
                publish("log_path", writer.path.to_string_lossy().into_owned());
            }
            #[cfg(windows)]
            let process =
                unsafe { windows_sys::Win32::System::Threading::OpenProcess(0x1000, 0, pid) };
            #[cfg(windows)]
            if process.is_null() {
                let error = format!(
                    "无法观察游戏进程，日志跟随已停止：{}",
                    std::io::Error::last_os_error()
                );
                log_output(&mut log, "error", &error);
                if let Some(writer) = log.as_mut() {
                    let _ = writer.flush();
                }
                publish("error", error);
                return;
            }
            let started = std::time::Instant::now();
            let mut failures = 0;
            loop {
                #[cfg(windows)]
                let running = unsafe {
                    // 使用有限查询权限观察提权游戏，避免为等待退出申请更高权限。
                    let mut code = 0;
                    windows_sys::Win32::System::Threading::GetExitCodeProcess(process, &mut code)
                        != 0
                        && code == 259
                };
                #[cfg(not(windows))]
                let running = false;
                match tail.read() {
                    Ok(text) => {
                        failures = 0;
                        if !text.is_empty() {
                            log_output(&mut log, "stdout", &text);
                            publish("stdout", text);
                        }
                    }
                    Err(error) => {
                        failures += 1;
                        if failures >= 20 {
                            let error = format!("日志读取失败：{error}");
                            log_output(&mut log, "error", &error);
                            publish("error", error);
                            break;
                        }
                    }
                }
                if let Some(writer) = log.as_mut() {
                    let _ = writer.flush();
                }
                if !running || started.elapsed() > Duration::from_secs(48 * 60 * 60) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(process);
            }
            // 游戏退出前可能刚写出多于一个分片；有界排空，避免丢掉最后的错误信息。
            for _ in 0..128 {
                match tail.read() {
                    Ok(text) if !text.is_empty() => {
                        log_output(&mut log, "stdout", &text);
                        publish("stdout", text);
                    }
                    _ => break,
                }
            }
            log_output(&mut log, "exit", "游戏进程结束或日志跟随停止");
            if let Some(writer) = log.as_mut() {
                let _ = writer.flush();
            }
            publish("exit", String::new());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn source() -> LaunchLogSource {
        LaunchLogSource {
            name: "ReShade".into(),
            path: std::env::temp_dir().join(format!("ssmt-log-tail-{}.log", nonce())),
        }
    }
    #[test]
    fn skips_previous_run_and_follows_creation_append_and_rewrite() {
        let source = source();
        let mut tail = LogTail::new(source.clone());
        assert_eq!(tail.read().unwrap(), "");
        std::fs::write(&source.path, "old run\n").unwrap();
        tail = LogTail::new(source.clone());
        assert_eq!(tail.read().unwrap(), "");
        OpenOptions::new()
            .append(true)
            .open(&source.path)
            .unwrap()
            .write_all(b"new line\n")
            .unwrap();
        assert_eq!(tail.read().unwrap(), "new line\n");
        assert_eq!(tail.read().unwrap(), "");
        // 重写后的文件比基线更大，不能仅靠长度判断是否轮转。
        std::fs::write(&source.path, "replacement run with longer first line\n").unwrap();
        assert_eq!(
            tail.read().unwrap(),
            "replacement run with longer first line\n"
        );
        std::fs::remove_file(&source.path).unwrap();
        assert_eq!(tail.read().unwrap(), "");
        std::fs::write(&source.path, "next\n").unwrap();
        assert_eq!(tail.read().unwrap(), "next\n");
        std::fs::remove_file(source.path).unwrap();
    }
    #[test]
    fn preserves_split_utf8_without_retaining_file_lock() {
        let source = source();
        let mut tail = LogTail::new(source.clone());
        let text = "原神日志\n".as_bytes();
        std::fs::write(&source.path, &text[..2]).unwrap();
        assert_eq!(tail.read().unwrap(), "");
        OpenOptions::new()
            .append(true)
            .open(&source.path)
            .unwrap()
            .write_all(&text[2..])
            .unwrap();
        assert_eq!(tail.read().unwrap(), "原神日志\n");
        std::fs::rename(&source.path, source.path.with_extension("old")).unwrap();
        std::fs::write(&source.path, "fresh").unwrap();
        assert_eq!(tail.read().unwrap(), "fresh");
        std::fs::remove_file(&source.path).unwrap();
        std::fs::remove_file(source.path.with_extension("old")).unwrap();
    }

    #[test]
    fn large_output_is_read_in_bounded_chunks_without_loss() {
        let source = source();
        let mut tail = LogTail::new(source.clone());
        let text = "日志大段输出\n".repeat(9000);
        std::fs::write(&source.path, &text).unwrap();
        let mut actual = String::new();
        for _ in 0..16 {
            let chunk = tail.read().unwrap();
            assert!(chunk.len() <= 32 * 1024 + 4);
            if chunk.is_empty() {
                break;
            }
            actual.push_str(&chunk);
        }
        assert_eq!(actual, text);
        std::fs::remove_file(source.path).unwrap();
    }
}
