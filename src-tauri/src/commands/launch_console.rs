use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const MAX_PACKET: usize = 64 * 1024;
const MAX_INPUT: usize = 4096;
const MAX_SESSIONS: usize = 16;

pub(super) struct ConsoleLog {
    file: Option<BufWriter<std::fs::File>>,
    pub(super) path: PathBuf,
    written: usize,
    limit: usize,
}

impl ConsoleLog {
    pub(super) fn new(folder: &Path, id: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(folder)?;
        let path = folder.join(format!(
            "{}-launch-console-{}.log",
            chrono::Local::now().format("%Y%m%dT%H%M%S%.3f"),
            id
        ));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            file: Some(BufWriter::new(file)),
            path,
            written: 0,
            limit: 4 * 1024 * 1024,
        })
    }

    fn append(&mut self, kind: &str, text: &str) -> std::io::Result<()> {
        if self.written >= self.limit {
            if let Some(mut file) = self.file.take() {
                file.flush()?;
            }
            let previous = self.path.with_extension("previous.log");
            match std::fs::remove_file(&previous) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            std::fs::rename(&self.path, previous)?;
            self.file = Some(BufWriter::new(std::fs::File::create(&self.path)?));
            self.written = 0;
        }
        let line = format!(
            "{} [{kind}] {text}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
        );
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("日志文件不可写"))?
            .write_all(line.as_bytes())?;
        self.written += line.len();
        Ok(())
    }

    pub(super) fn flush(&mut self) -> std::io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("日志文件不可写"))?
            .flush()
    }
}

pub(super) fn log_output(log: &mut Option<ConsoleLog>, kind: &str, text: &str) {
    if let Some(writer) = log.as_mut() {
        if let Err(error) = writer.append(kind, text) {
            eprintln!("[LaunchConsole] 日志写入失败：{error}");
            *log = None;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleRequest {
    pub path: String,
    pub args: String,
    pub work_dir: String,
    #[serde(default)]
    pub log_sources: Vec<super::launch_logs::LaunchLogSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleEvent {
    pub session_id: String,
    pub program: String,
    pub kind: String,
    pub text: String,
    pub pid: Option<u32>,
    pub code: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Packet {
    kind: String,
    #[serde(default)]
    bytes: Vec<u8>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    code: Option<i32>,
}

type Sessions = Mutex<HashMap<String, Arc<Mutex<TcpStream>>>>;
fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(Mutex::default)
}

pub(super) fn nonce() -> String {
    let random = std::collections::hash_map::RandomState::new();
    format!("{:016x}{:016x}", random.hash_one(0u8), random.hash_one(1u8))
}

fn read_json<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<T, String> {
    let mut line = Vec::new();
    reader
        .take((MAX_PACKET + 1) as u64)
        .read_until(b'\n', &mut line)
        .map_err(|error| error.to_string())?;
    if line.len() > MAX_PACKET || line.last() != Some(&b'\n') {
        return Err("控制台通道已关闭或消息过长".into());
    }
    serde_json::from_slice(&line).map_err(|error| error.to_string())
}

fn write_json(writer: &mut impl Write, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if bytes.len() >= MAX_PACKET {
        return Err("控制台消息过长".into());
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).map_err(|error| error.to_string())
}

fn set_reader_timeout(
    stream: &TcpStream,
    reader: &BufReader<TcpStream>,
    timeout: Option<Duration>,
) -> Result<(), String> {
    // 超时必须设到实际 read 所用的句柄，不能依赖 try_clone 在各平台传播选项。
    stream
        .set_read_timeout(timeout)
        .map_err(|error| error.to_string())?;
    reader
        .get_ref()
        .set_read_timeout(timeout)
        .map_err(|error| error.to_string())
}

// 只路由确定的 Console PE 与批处理；GUI 启动器继续使用原来的 Shell/UAC 路径。
pub fn is_console_program(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "cmd" || ext == "bat" {
        return true;
    }
    if ext != "exe" {
        return false;
    }
    (|| -> std::io::Result<bool> {
        let mut file = std::fs::File::open(path)?;
        let mut dos = [0; 64];
        file.read_exact(&mut dos)?;
        if &dos[..2] != b"MZ" {
            return Ok(false);
        }
        let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
        if offset > file.metadata()?.len().saturating_sub(94) {
            return Ok(false);
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0; 94];
        file.read_exact(&mut header)?;
        let magic = u16::from_le_bytes([header[24], header[25]]);
        Ok(&header[..4] == b"PE\0\0"
            && matches!(magic, 0x10b | 0x20b)
            && u16::from_le_bytes([header[92], header[93]]) == 3)
    })()
    .unwrap_or(false)
}

#[cfg(windows)]
fn child_command(request: &ConsoleRequest) -> Command {
    let path = Path::new(&request.path);
    let is_batch = path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("cmd") || v.eq_ignore_ascii_case("bat"));
    let mut command = if is_batch {
        let mut command =
            Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
        command.args(["/d", "/s", "/c"]);
        command.raw_arg(format!("\"\"{}\" {}\"", request.path, request.args));
        command
    } else {
        let mut command = Command::new(path);
        // 已有配置是完整 Windows 命令行，不能按空格再次拆分。
        command.raw_arg(&request.args);
        command
    };
    command
        .current_dir(&request.work_dir)
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(not(windows))]
fn child_command(request: &ConsoleRequest) -> Command {
    let mut command = Command::new(&request.path);
    command
        .current_dir(&request.work_dir)
        .args(request.args.split_whitespace())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn pump(
    mut reader: impl Read + Send + 'static,
    kind: &'static str,
    sender: mpsc::SyncSender<Packet>,
) {
    std::thread::spawn(move || {
        let mut bytes = [0u8; 2048];
        loop {
            let length = match reader.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if sender
                .send(Packet {
                    kind: kind.into(),
                    bytes: bytes[..length].to_vec(),
                    pid: None,
                    code: None,
                })
                .is_err()
            {
                break;
            }
        }
    });
}

// 单次连接只接受本次父进程提供的命令。提权后在同级权限内建立 stdio 管道，
// 不依赖 ShellExecute 继承句柄，也不写入可被复用的提权请求文件。
pub fn console_helper(port: u16, token: &str) -> Result<(), String> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_secs(5),
    )
    .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    write_json(&mut stream, &token)?;
    let mut input = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let request: ConsoleRequest = read_json(&mut input)?;
    set_reader_timeout(&stream, &input, None)?;
    let mut child = match child_command(&request).spawn() {
        Ok(child) => child,
        Err(error) => {
            write_json(
                &mut stream,
                &Packet {
                    kind: "error".into(),
                    bytes: error.to_string().into_bytes(),
                    pid: None,
                    code: None,
                },
            )?;
            return Err(error.to_string());
        }
    };
    // 连接断开只关闭工具 stdin，不能以关闭 SSMT 或浮窗为由结束工具及游戏。
    let mut stdin = child.stdin.take().unwrap();
    std::thread::spawn(move || {
        while let Ok(text) = read_json::<String>(&mut input) {
            if text.len() > MAX_INPUT
                || stdin
                    .write_all(text.as_bytes())
                    .and_then(|_| stdin.flush())
                    .is_err()
            {
                break;
            }
        }
    });
    let (sender, receiver) = mpsc::sync_channel(32);
    pump(child.stdout.take().unwrap(), "stdout", sender.clone());
    pump(child.stderr.take().unwrap(), "stderr", sender.clone());
    drop(sender);
    let mut connected = write_json(
        &mut stream,
        &Packet {
            kind: "started".into(),
            bytes: vec![],
            pid: Some(child.id()),
            code: None,
        },
    )
    .is_ok();
    let mut exited = None;
    let mut exit_time = None;
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(packet) => {
                if connected && write_json(&mut stream, &packet).is_err() {
                    connected = false;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) if exited.is_some() => break,
            _ => {}
        }
        if exited.is_none() {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                exited = Some(status.code().unwrap_or(-1));
                exit_time = Some(Instant::now());
            }
        }
        // 后代可能仍持有管道：最多排空两秒，不能让已退出工具的会话永久挂住。
        if exit_time.is_some_and(|time| time.elapsed() >= Duration::from_secs(2)) {
            break;
        }
    }
    if connected {
        let _ = write_json(
            &mut stream,
            &Packet {
                kind: "exit".into(),
                bytes: vec![],
                pid: Some(child.id()),
                code: exited,
            },
        );
    }
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

pub(super) struct StreamText {
    pending: Vec<u8>,
    legacy: Option<encoding_rs::Decoder>,
}
impl StreamText {
    pub(super) fn new() -> Self {
        Self {
            pending: vec![],
            legacy: None,
        }
    }
    pub(super) fn decode(&mut self, bytes: &[u8], eof: bool) -> String {
        self.pending.extend_from_slice(bytes);
        if let Some(decoder) = self.legacy.as_mut() {
            let mut text = String::with_capacity(self.pending.len() * 3 + 8);
            let _ = decoder.decode_to_string(&self.pending, &mut text, eof);
            self.pending.clear();
            return text;
        }
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_owned();
                self.pending.clear();
                text
            }
            Err(error) if error.error_len().is_none() && !eof => {
                let text =
                    String::from_utf8_lossy(&self.pending[..error.valid_up_to()]).into_owned();
                self.pending.drain(..error.valid_up_to());
                text
            }
            Err(_) => {
                self.legacy = Some(encoding_rs::GBK.new_decoder());
                self.decode(&[], eof)
            }
        }
    }
}

fn helper_script(exe: &Path, port: u16, token: &str, elevated: bool) -> String {
    format!("$ErrorActionPreference='Stop'; try {{ Start-Process -FilePath '{}' -WindowStyle Hidden {} -ArgumentList '--ssmt-launch-console {} {}' }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}",
        exe.to_string_lossy().replace('\'', "''"), if elevated { "-Verb RunAs" } else { "" }, port, token)
}

fn accept_helper(
    listener: &TcpListener,
    token: &str,
) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Instant::now() >= deadline {
            return Err("控制台连接超时，未重复启动程序".into());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // Windows 的 accept 会继承监听端非阻塞状态；超时读取前必须复原。
                stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .map_err(|e| e.to_string())?;
                let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
                if read_json::<String>(&mut reader).is_ok_and(|value| value == token) {
                    return Ok((stream, reader));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub fn launch(app: AppHandle, request: ConsoleRequest, elevated: bool) -> Result<(), String> {
    if sessions().lock().map_err(|e| e.to_string())?.len() >= MAX_SESSIONS {
        return Err("同时运行的控制台程序过多，请等待现有工具退出".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = nonce();
    let id = nonce();
    let mut log_sources: Option<Vec<_>> = Some(
        request.log_sources.iter().take(4).cloned()
            .map(super::launch_logs::LogTail::new).collect(),
    );
    let log_folder = crate::config::path_manager::PathManager::ssmt_cache_root(&app).join("Logs");
    let mut log = match ConsoleLog::new(&log_folder, &id) {
        Ok(log) => Some(log),
        Err(error) => {
            eprintln!("[LaunchConsole] 无法创建回显日志：{error}");
            None
        }
    };
    log_output(
        &mut log,
        "launch",
        &format!(
            "program={} work_dir={} elevated={elevated}",
            request.path, request.work_dir
        ),
    );
    if let Some(log) = log.as_mut() {
        let _ = log.flush();
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = helper_script(&exe, port, &token, elevated);
    let mut launcher = Command::new("powershell");
    launcher.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    #[cfg(windows)]
    launcher.creation_flags(0x08000000);
    let result = launcher
        .output()
        .map_err(|e| format!("控制台启动失败：{e}"))?;
    if !result.status.success() {
        log_output(&mut log, "error", &String::from_utf8_lossy(&result.stderr));
        return Err(format!(
            "控制台启动失败或提权被取消：{}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let (mut stream, mut reader) = accept_helper(&listener, &token).map_err(|error| {
        log_output(&mut log, "error", &error);
        error
    })?;
    set_reader_timeout(&stream, &reader, Some(Duration::from_secs(15)))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    write_json(&mut stream, &request)?;
    let started: Packet = match read_json(&mut reader) {
        Ok(packet) => packet,
        Err(error) => {
            log_output(&mut log, "unconfirmed", &error);
            // 请求已经交给 helper，目标可能已启动；此后不能当作启动失败回滚或重试。
            let program = Path::new(&request.path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            for (kind, text) in [
                ("started", String::new()),
                (
                    "error",
                    format!(
                        "程序已提交启动，控制台状态未确认：{error}。请检查游戏，不要重复启动。"
                    ),
                ),
            ] {
                let _ = app.emit(
                    "launch-console-event",
                    ConsoleEvent {
                        session_id: id.clone(),
                        program: program.clone(),
                        kind: kind.into(),
                        text,
                        pid: None,
                        code: None,
                    },
                );
            }
            return Ok(());
        }
    };
    if started.kind != "started" {
        log_output(&mut log, "error", &String::from_utf8_lossy(&started.bytes));
        return Err(String::from_utf8_lossy(&started.bytes).into_owned());
    }
    log_output(&mut log, "started", &format!("pid={:?}", started.pid));
    set_reader_timeout(&stream, &reader, None)?;
    log_output(
        &mut log,
        "channel",
        &format!("reader_timeout={:?}", reader.get_ref().read_timeout()),
    );
    sessions()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id.clone(), Arc::new(Mutex::new(stream)));
    let program = Path::new(&request.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let session_id = id.clone();
    let event_app = app.clone();
    let machine_events =
        request.args.contains("--machine-readable") && program.eq_ignore_ascii_case("Run.exe");
    let publish = move |kind: &str, text: String, pid: Option<u32>, code: Option<i32>| {
        let _ = app.emit(
            "launch-console-event",
            ConsoleEvent {
                session_id: id.clone(),
                program: program.clone(),
                kind: kind.into(),
                text,
                pid,
                code,
            },
        );
    };
    publish("started", String::new(), started.pid, None);
    if let Some(log) = log.as_mut() {
        let _ = log.flush();
        publish(
            "log_path",
            log.path.to_string_lossy().into_owned(),
            started.pid,
            None,
        );
    }
    // 监听线程始终读取；界面隐藏不导致工具输出管道堵塞。
    std::thread::spawn(move || {
        let (packets, receiver) = mpsc::sync_channel(64);
        std::thread::spawn(move || loop {
            match read_json::<Packet>(&mut reader) {
                Ok(packet) => {
                    if packets.send(packet).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = packets.send(Packet {
                        kind: "channel_error".into(),
                        bytes: error.into_bytes(),
                        pid: None,
                        code: None,
                    });
                    break;
                }
            }
        });
        let mut out = StreamText::new();
        let mut err = StreamText::new();
        let mut lines = String::new();
        let mut pending_out = String::new();
        let mut pending_err = String::new();
        let mut last_flush = Instant::now();
        let mut omitted = false;
        let mut ended = false;
        loop {
            let received = receiver.recv_timeout(Duration::from_millis(50));
            if last_flush.elapsed() >= Duration::from_millis(100)
                || matches!(received, Ok(Packet { ref kind, .. }) if kind == "exit")
                || matches!(received, Err(mpsc::RecvTimeoutError::Disconnected))
            {
                for (kind, text) in [("stdout", &mut pending_out), ("stderr", &mut pending_err)] {
                    if !text.is_empty() {
                        publish(kind, std::mem::take(text), None, None);
                    }
                }
                if omitted {
                    publish("truncated", String::new(), None, None);
                    omitted = false;
                }
                if let Some(writer) = log.as_mut() {
                    if let Err(error) = writer.flush() {
                        eprintln!("[LaunchConsole] 日志刷新失败：{error}");
                        log = None;
                    }
                }
                last_flush = Instant::now();
            }
            let packet = match received {
                Ok(packet) => packet,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if packet.kind == "stdout" || packet.kind == "stderr" {
                let decoder = if packet.kind == "stdout" {
                    &mut out
                } else {
                    &mut err
                };
                let text = decoder.decode(&packet.bytes, false);
                // 日志记录限流前的回显；界面截断不能丢掉诊断证据。
                if !text.is_empty() {
                    log_output(&mut log, &packet.kind, &text);
                }
                if machine_events && packet.kind == "stdout" {
                    lines.push_str(&text);
                    while let Some(end) = lines.find('\n') {
                        if let Ok(Some(mut event)) =
                            crate::plugin::launch_event_bus::parse_json_line(&lines[..end])
                        {
                            if matches!(event.event.as_str(), "target_created" | "target_detected")
                                && event.pid.is_some_and(|pid| pid > 0) {
                                if let (Some(pid), Some(sources)) = (event.pid, log_sources.take()) {
                                    super::launch_logs::follow(event_app.clone(), sources, pid, &session_id);
                                }
                            }
                            event
                                .details
                                .insert("consoleSessionId".into(), session_id.clone().into());
                            let _ = crate::plugin::launch_event_bus::LaunchEventBus::publish(
                                &event_app, &event,
                            );
                        }
                        lines.drain(..=end);
                    }
                    if lines.len() > MAX_PACKET {
                        lines.clear();
                    }
                }
                let pending = if packet.kind == "stdout" {
                    &mut pending_out
                } else {
                    &mut pending_err
                };
                pending.push_str(&text);
                if pending.len() > 16 * 1024 {
                    let mut start = pending.len() - 16 * 1024;
                    while !pending.is_char_boundary(start) {
                        start += 1;
                    }
                    pending.drain(..start);
                    omitted = true;
                }
            } else if packet.kind == "channel_error" {
                let message = format!(
                    "控制台连接断开；程序可能仍在运行。原因：{}",
                    String::from_utf8_lossy(&packet.bytes)
                );
                log_output(&mut log, "error", &message);
                publish("error", message, None, None);
                ended = true;
                break;
            } else {
                if packet.kind == "exit" {
                    ended = true;
                }
                for (kind, decoder) in [("stdout", &mut out), ("stderr", &mut err)] {
                    let tail = decoder.decode(&[], true);
                    if !tail.is_empty() {
                        log_output(&mut log, kind, &tail);
                        publish(kind, tail, None, None);
                    }
                }
                log_output(
                    &mut log,
                    &packet.kind,
                    &format!(
                        "pid={:?} code={:?} {}",
                        packet.pid,
                        packet.code,
                        String::from_utf8_lossy(&packet.bytes)
                    ),
                );
                publish(
                    &packet.kind,
                    String::from_utf8_lossy(&packet.bytes).into_owned(),
                    packet.pid,
                    packet.code,
                );
                if ended {
                    break;
                }
            }
        }
        if !ended {
            log_output(&mut log, "error", "控制台连接断开；程序可能仍在运行");
            publish(
                "error",
                "控制台连接断开；程序可能仍在运行".into(),
                None,
                None,
            );
        }
        if let Some(log) = log.as_mut() {
            let _ = log.flush();
        }
        if let Ok(mut registry) = sessions().lock() {
            registry.remove(&session_id);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn write_launch_console(session_id: String, text: String) -> Result<(), String> {
    if text.len() > MAX_INPUT - 2 {
        return Err("单次输入不能超过 4094 字节".into());
    }
    let stream = sessions()
        .lock()
        .map_err(|e| e.to_string())?
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "控制台程序已退出或会话不存在".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut stream = stream.lock().map_err(|e| e.to_string())?;
        write_json(&mut *stream, &format!("{text}\r\n"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_nonblocking_listener_waits_for_delayed_packet() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut wrong = TcpStream::connect(address).unwrap();
            write_json(&mut wrong, &"wrong-token").unwrap();
            drop(wrong);
            let mut stream = TcpStream::connect(address).unwrap();
            write_json(&mut stream, &"test-token").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            write_json(&mut stream, &"delayed-packet").unwrap();
        });
        let (_stream, mut reader) = accept_helper(&listener, "test-token").unwrap();
        // 真实 Winsock 延迟读：若 accept 后未清非阻塞状态，此处复现 10035。
        assert_eq!(read_json::<String>(&mut reader).unwrap(), "delayed-packet");
        thread.join().unwrap();
    }

    #[test]
    fn idle_console_read_has_no_handshake_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            write_json(&mut stream, &"idle-token").unwrap();
            std::thread::sleep(Duration::from_millis(1400));
            write_json(&mut stream, &"after-idle").unwrap();
        });
        let (stream, mut reader) = accept_helper(&listener, "idle-token").unwrap();
        stream.set_read_timeout(None).unwrap();
        eprintln!(
            "original={:?}, cloned reader={:?}",
            stream.read_timeout(),
            reader.get_ref().read_timeout()
        );
        set_reader_timeout(&stream, &reader, None).unwrap();
        assert_eq!(reader.get_ref().read_timeout().unwrap(), None);
        assert_eq!(read_json::<String>(&mut reader).unwrap(), "after-idle");
        thread.join().unwrap();
    }

    #[test]
    fn stream_text_preserves_utf8_and_legacy_multibyte_boundaries() {
        let mut utf8 = StreamText::new();
        let mut text = String::new();
        for byte in "中文🚀\n".as_bytes() {
            text.push_str(&utf8.decode(&[*byte], false));
        }
        text.push_str(&utf8.decode(&[], true));
        assert_eq!(text, "中文🚀\n");
        let mut legacy = StreamText::new();
        let (bytes, _, _) = encoding_rs::GBK.encode("你好世界");
        let mut text = String::new();
        for byte in bytes.iter() {
            text.push_str(&legacy.decode(&[*byte], false));
        }
        text.push_str(&legacy.decode(&[], true));
        assert_eq!(text, "你好世界");
    }

    #[test]
    fn oversized_and_incomplete_packets_are_rejected() {
        assert!(read_json::<String>(&mut &b"\"partial"[..]).is_err());
        assert!(read_json::<String>(&mut vec![b'a'; MAX_PACKET + 2].as_slice()).is_err());
    }

    #[test]
    fn output_log_rotates_and_keeps_stream_pid_and_exit_records() {
        let root = std::env::temp_dir().join(format!("ssmt-console-log-{}", nonce()));
        let mut writer = ConsoleLog::new(&root, "fixture").unwrap();
        writer.limit = 100;
        writer
            .append("started", "program=fixture.exe pid=123")
            .unwrap();
        writer.append("stdout", "prompt>中文").unwrap();
        writer.append("stderr", "stderr-marker").unwrap();
        writer.append("exit", "pid=123 code=7").unwrap();
        writer.flush().unwrap();
        let current = std::fs::read_to_string(&writer.path).unwrap();
        let previous = std::fs::read_to_string(writer.path.with_extension("previous.log")).unwrap();
        let all = current + &previous;
        assert!(all.contains("[stdout] prompt>中文"));
        assert!(all.contains("[stderr] stderr-marker"));
        assert!(all.contains("[started] program=fixture.exe pid=123"));
        assert!(all.contains("[exit] pid=123 code=7"));
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn helper_arguments_keep_quotes_and_uac_out_of_program_arguments() {
        let script = helper_script(
            Path::new("C:\\用户's files\\SSMT.exe"),
            4321,
            "abcdef",
            true,
        );
        assert!(script.contains("用户''s files"));
        assert!(script.contains("-WindowStyle Hidden -Verb RunAs"));
        assert!(script.contains("--ssmt-launch-console 4321 abcdef"));
        assert!(!helper_script(Path::new("SSMT.exe"), 4321, "abcdef", false).contains("RunAs"));
    }

    #[cfg(windows)]
    fn powershell_request(script: &str) -> ConsoleRequest {
        use base64::Engine;
        let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let script = base64::engine::general_purpose::STANDARD.encode(utf16);
        ConsoleRequest {
            path: std::env::var("SystemRoot").unwrap()
                + "\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
            args: format!("-NoProfile -NonInteractive -EncodedCommand {script}"),
            work_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            log_sources: Vec::new(),
        }
    }

    #[cfg(windows)]
    #[test]
    fn helper_captures_prompt_stderr_stdin_large_output_and_exit_code() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let helper = std::thread::spawn(move || console_helper(port, "fixture"));
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert_eq!(read_json::<String>(&mut reader).unwrap(), "fixture");
        let request = powershell_request("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::InputEncoding = [Text.UTF8Encoding]::new($false); [Console]::Write('prompt>'); $line=[Console]::ReadLine(); [Console]::WriteLine('echo='+$line); [Console]::Error.Write('stderr-marker'); [Console]::Write(('x'*200000)); exit 7");
        assert!(is_console_program(Path::new(&request.path)));
        write_json(&mut stream, &request).unwrap();
        let started: Packet = read_json(&mut reader).unwrap();
        assert_eq!(started.kind, "started");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut input_sent = false;
        loop {
            let packet: Packet = read_json(&mut reader).unwrap();
            match packet.kind.as_str() {
                "stdout" => stdout.extend_from_slice(&packet.bytes),
                "stderr" => stderr.extend_from_slice(&packet.bytes),
                "exit" => {
                    assert_eq!(packet.code, Some(7));
                    break;
                }
                other => panic!("unexpected packet {other}"),
            }
            if !input_sent && String::from_utf8_lossy(&stdout).contains("prompt>") {
                // 长时间不输入也不能遗留 helper 的十秒握手超时，提前关闭工具 stdin。
                std::thread::sleep(Duration::from_secs(11));
                write_json(&mut stream, &"中文输入\r\n").unwrap();
                input_sent = true;
            }
        }
        assert!(String::from_utf8_lossy(&stdout).contains("echo=中文输入"));
        assert!(stdout.len() >= 200_000);
        assert_eq!(String::from_utf8_lossy(&stderr), "stderr-marker");
        helper.join().unwrap().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn helper_disconnect_does_not_kill_launched_program() {
        let marker = std::env::temp_dir().join(format!("ssmt-console-{}.txt", nonce()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let helper = std::thread::spawn(move || console_helper(port, "disconnect"));
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let _: String = read_json(&mut reader).unwrap();
        let script = format!(
            "Start-Sleep -Milliseconds 300; [IO.File]::WriteAllText('{}', 'survived')",
            marker.to_string_lossy().replace('\'', "''")
        );
        write_json(&mut stream, &powershell_request(&script)).unwrap();
        assert_eq!(read_json::<Packet>(&mut reader).unwrap().kind, "started");
        stream.shutdown(Shutdown::Both).unwrap();
        drop(reader);
        drop(stream);
        helper.join().unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "survived");
        std::fs::remove_file(marker).unwrap();
    }
}
