use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    net::TcpListener,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::State;

const PLUGIN_ID: &str = "ssmt.liquid-gravity-probe";
const SUPPORTED_GAME: &str = "EFMI";
const HEALTH_SERVICE: &str = "liquid-gravity-probe-editor";
const HEALTH_PROTOCOL_VERSION: u32 = 1;
const START_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_TIMEOUT_MS: u64 = 700;
const MAX_HEALTH_MISSES: u8 = 3;
const LOG_TAIL_LINES: usize = 18;
const GENERATED_PROBE_MOD: &str = "SSMT_LiquidGravityProbe";
const PYTHON_DEPENDENCY_ID: &str = "python-runtime";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static INSTANCE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct RunningEditor {
    plugin_root: PathBuf,
    editor_root: PathBuf,
    data_root: PathBuf,
    port: u16,
    instance_token: String,
    log_path: PathBuf,
    health_misses: u8,
    child: Child,
}

pub struct LiquidGravityProbeProcess(Mutex<Option<RunningEditor>>);

impl Default for LiquidGravityProbeProcess {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl Drop for LiquidGravityProbeProcess {
    fn drop(&mut self) {
        if let Ok(mut process) = self.0.lock() {
            if let Some(editor) = process.as_mut() {
                let _ = stop_child(&mut editor.child);
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidGravityProbeEditorInfo {
    pub editor_root: String,
    pub data_root: String,
    pub running: bool,
    pub url: String,
    pub api_token: String,
    pub log_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditorHealth {
    service: String,
    protocol_version: u32,
    instance_token: String,
}

enum EditorStatus {
    Healthy,
    Unhealthy,
    Stopped,
}

struct PythonRuntime {
    program: PathBuf,
    launcher_args: Vec<OsString>,
}

fn enabled_plugin_context(game_name: &str) -> Result<(PathBuf, Option<PathBuf>), String> {
    if !game_name.trim().eq_ignore_ascii_case(SUPPORTED_GAME) {
        return Err("Liquid Gravity Probe 仅支持 EFMI 游戏配置".to_string());
    }
    let registry = crate::plugin::registry::PluginRegistry::from_default_location()
        .map_err(|error| format!("读取 SSMT 插件状态失败：{error}"))?;
    let scoped = registry.for_game(game_name);
    let plugin = scoped
        .find(PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
        .ok_or_else(|| "请先在插件市场安装并启用 Liquid Gravity Probe".to_string())?;
    let python_root = plugin
        .external_dependencies
        .get(PYTHON_DEPENDENCY_ID)
        .and_then(|dependency| dependency.path.clone());
    Ok((plugin.package_root.clone(), python_root))
}

fn bundled_editor_root(plugin_root: &Path) -> Result<PathBuf, String> {
    let editor_root = plugin_root.join("editor");
    if !editor_root.join("server.py").is_file() {
        return Err(format!("插件缺少内置编辑器源码：{}", editor_root.display()));
    }
    Ok(editor_root)
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("无法创建编辑器目录 {}：{error}", destination.display()))?;
    for entry in
        fs::read_dir(source).map_err(|error| format!("无法读取 {}：{error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("读取目录项失败：{error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("读取文件类型失败：{error}"))?
            .is_dir()
        {
            copy_dir_all(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "无法复制 {} 到 {}：{error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn directories_match(source: &Path, destination: &Path) -> bool {
    if !source.is_dir() || !destination.is_dir() {
        return false;
    }
    let mut stack = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source_dir, destination_dir)) = stack.pop() {
        let Ok(source_entries) = fs::read_dir(&source_dir) else {
            return false;
        };
        let Ok(destination_entries) = fs::read_dir(&destination_dir) else {
            return false;
        };
        let source_count = source_entries.count();
        let destination_count = destination_entries.count();
        if source_count != destination_count {
            return false;
        }
        let Ok(source_entries) = fs::read_dir(&source_dir) else {
            return false;
        };
        for entry in source_entries.flatten() {
            let source_path = entry.path();
            let destination_path = destination_dir.join(entry.file_name());
            let Ok(source_type) = entry.file_type() else {
                return false;
            };
            let Ok(destination_type) = fs::metadata(&destination_path).map(|m| m.is_dir()) else {
                return false;
            };
            if source_type.is_dir() != destination_type {
                return false;
            }
            if source_type.is_dir() {
                stack.push((source_path, destination_path));
            } else if fs::read(&source_path).ok() != fs::read(&destination_path).ok() {
                return false;
            }
        }
    }
    true
}

fn materialize_editor_payload(plugin_root: &Path, data_root: &Path) -> Result<PathBuf, String> {
    let source_editor = bundled_editor_root(plugin_root)?;
    let editor_root = data_root.join("Editor");
    let source_ui = plugin_root.join("ui");
    let ui_root = data_root.join("ui");
    let legacy_runtime_root = data_root.join("Runtime");
    let marker = data_root.join(".editor-payload");
    let expected_marker = plugin_root.to_string_lossy();
    let marker_matches = fs::read_to_string(&marker).ok().as_deref() == Some(expected_marker.as_ref());
    let editor_ready = marker_matches && directories_match(&source_editor, &editor_root);
    let ui_ready = source_ui.join("index.html").is_file() && directories_match(&source_ui, &ui_root);
    if !editor_ready || !ui_ready {
        fs::create_dir_all(data_root)
            .map_err(|error| format!("无法创建插件 Mod 数据目录：{error}"))?;
        if !editor_root.starts_with(data_root) {
            return Err("拒绝写入插件 Mod 数据目录之外的路径".to_string());
        }
        if !editor_ready {
            if editor_root.exists() {
                fs::remove_dir_all(&editor_root)
                    .map_err(|error| format!("无法更新 {}：{error}", editor_root.display()))?;
            }
            copy_dir_all(&source_editor, &editor_root)?;
        }
        if source_ui.join("index.html").is_file() && !ui_ready {
            if ui_root.exists() {
                fs::remove_dir_all(&ui_root)
                    .map_err(|error| format!("无法更新 {}：{error}", ui_root.display()))?;
            }
            copy_dir_all(&source_ui, &ui_root)?;
        }
        fs::write(&marker, expected_marker.as_bytes())
            .map_err(|error| format!("无法写入编辑器载荷标记：{error}"))?;
    }
    if legacy_runtime_root.is_dir() {
        fs::remove_dir_all(&legacy_runtime_root).map_err(|error| {
            format!(
                "无法清理旧内置运行时 {}：{error}",
                legacy_runtime_root.display()
            )
        })?;
    }
    Ok(editor_root)
}

fn mods_root(migoto_root: &Path) -> PathBuf {
    migoto_root.join("Mods")
}

fn probe_data_root(migoto_root: &Path) -> PathBuf {
    mods_root(migoto_root).join(GENERATED_PROBE_MOD)
}

fn log_path(data_root: &Path) -> PathBuf {
    data_root.join(".logs").join("editor.log")
}

fn instance_token() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = INSTANCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}-{:x}", std::process::id(), timestamp, sequence)
}

fn choose_free_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("无法分配 Liquid Gravity Probe 本地端口：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("无法读取 Liquid Gravity Probe 本地端口：{error}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn python_command(runtime: &PythonRuntime) -> Command {
    let mut command = Command::new(&runtime.program);
    command
        .args(&runtime.launcher_args)
        .creation_flags(CREATE_NO_WINDOW);
    command
}

fn validate_python(runtime: &PythonRuntime) -> Result<(), String> {
    let output = python_command(runtime)
        .arg("-c")
        .arg("import sys; raise SystemExit(0 if sys.version_info >= (3, 12) else 3)")
        .output()
        .map_err(|error| format!("无法启动 Python：{error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        "需要 Python 3.12 或更高版本".to_string()
    } else {
        format!("需要 Python 3.12 或更高版本：{stderr}")
    })
}

fn configured_python(root: Option<&Path>) -> Option<PythonRuntime> {
    let root = root?;
    for name in ["python.exe", "pythonw.exe"] {
        let program = root.join(name);
        if program.is_file() {
            return Some(PythonRuntime {
                program,
                launcher_args: Vec::new(),
            });
        }
    }
    None
}

fn shortcut_python() -> Option<PythonRuntime> {
    let program = std::env::var_os("SSMT4_PYTHON").map(PathBuf::from)?;
    program.is_file().then_some(PythonRuntime {
        program,
        launcher_args: Vec::new(),
    })
}

fn path_python(name: &OsStr) -> Option<PythonRuntime> {
    let runtime = PythonRuntime {
        program: PathBuf::from(name),
        launcher_args: Vec::new(),
    };
    validate_python(&runtime).ok().map(|_| runtime)
}

fn py_launcher_python() -> Option<PythonRuntime> {
    let launcher = PythonRuntime {
        program: PathBuf::from("py.exe"),
        launcher_args: vec![OsString::from("-3")],
    };
    validate_python(&launcher).ok()?;
    let output = python_command(&launcher)
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let program = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    program.is_file().then_some(PythonRuntime {
        program,
        launcher_args: Vec::new(),
    })
}

fn discover_python(configured_root: Option<&Path>) -> Result<PythonRuntime, String> {
    let configured = configured_python(configured_root);
    if let Some(runtime) = configured {
        validate_python(&runtime)?;
        return Ok(runtime);
    }
    if let Some(runtime) = shortcut_python() {
        validate_python(&runtime)?;
        return Ok(runtime);
    }
    if let Some(runtime) = path_python(OsStr::new("python.exe"))
        .or_else(|| path_python(OsStr::new("python")))
        .or_else(py_launcher_python)
    {
        return Ok(runtime);
    }
    Err(
        "未找到 Python 3.12+。请安装官方 Python，然后在插件市场的“外部依赖”中选择 Python 安装目录。"
            .to_string(),
    )
}

fn start_external_editor(
    editor_root: &Path,
    python: &PythonRuntime,
    mods_root: &Path,
    data_root: &Path,
    output_root: &Path,
    port: u16,
    instance_token: &str,
    log_path: &Path,
) -> Result<Child, String> {
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("无法打开编辑器日志 {}：{error}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("无法复制编辑器日志句柄：{error}"))?;
    python_command(python)
        .arg(editor_root.join("server.py"))
        .arg("--mods")
        .arg(mods_root)
        .arg("--probe")
        .arg(data_root)
        .arg("--output")
        .arg(output_root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(editor_root)
        .env("SSMT_LGP_TOKEN", instance_token)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("启动 Python 编辑器失败：{error}"))
}

fn stop_child(child: &mut Child) -> Result<(), String> {
    if child
        .try_wait()
        .map_err(|error| format!("读取编辑器进程状态失败：{error}"))?
        .is_none()
    {
        child
            .kill()
            .map_err(|error| format!("停止 Python 编辑器失败：{error}"))?;
    }
    child
        .wait()
        .map_err(|error| format!("等待 Python 编辑器退出失败：{error}"))?;
    Ok(())
}

fn health_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(HEALTH_TIMEOUT_MS))
        .build()
        .map_err(|error| format!("无法创建编辑器健康检查客户端：{error}"))
}

fn is_editor_healthy(client: &reqwest::blocking::Client, port: u16, expected_token: &str) -> bool {
    let Ok(response) = client
        .get(format!("http://127.0.0.1:{port}/api/health"))
        .header("X-Editor-Token", expected_token)
        .send()
    else {
        return false;
    };
    let Ok(health) = response.json::<EditorHealth>() else {
        return false;
    };
    health.service == HEALTH_SERVICE
        && health.protocol_version == HEALTH_PROTOCOL_VERSION
        && health.instance_token == expected_token
}

fn refresh_running_state(slot: &mut Option<RunningEditor>) -> EditorStatus {
    let Some(editor) = slot.as_mut() else {
        return EditorStatus::Stopped;
    };
    if editor.child.try_wait().ok().flatten().is_some() {
        *slot = None;
        return EditorStatus::Stopped;
    }
    let healthy = health_client()
        .ok()
        .is_some_and(|client| is_editor_healthy(&client, editor.port, &editor.instance_token));
    if healthy {
        editor.health_misses = 0;
        return EditorStatus::Healthy;
    }
    editor.health_misses = editor.health_misses.saturating_add(1);
    if editor.health_misses < MAX_HEALTH_MISSES {
        return EditorStatus::Unhealthy;
    }
    let _ = stop_child(&mut editor.child);
    *slot = None;
    EditorStatus::Stopped
}

fn running_info(editor: &RunningEditor) -> LiquidGravityProbeEditorInfo {
    LiquidGravityProbeEditorInfo {
        editor_root: editor.editor_root.to_string_lossy().into_owned(),
        data_root: editor.data_root.to_string_lossy().into_owned(),
        running: true,
        url: format!("http://127.0.0.1:{}/", editor.port),
        api_token: editor.instance_token.clone(),
        log_path: editor.log_path.to_string_lossy().into_owned(),
    }
}

fn stopped_info(
    editor_root: &Path,
    data_root: &Path,
    log_path: &Path,
) -> LiquidGravityProbeEditorInfo {
    LiquidGravityProbeEditorInfo {
        editor_root: editor_root.to_string_lossy().into_owned(),
        data_root: data_root.to_string_lossy().into_owned(),
        running: false,
        url: String::new(),
        api_token: String::new(),
        log_path: log_path.to_string_lossy().into_owned(),
    }
}

fn log_tail(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines = text.lines().rev().take(LOG_TAIL_LINES).collect::<Vec<_>>();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn startup_error(reason: &str, log_path: &Path) -> String {
    let tail = log_tail(log_path);
    if tail.is_empty() {
        format!("{reason}。日志：{}", log_path.display())
    } else {
        format!("{reason}。日志：{}\n最近输出：\n{tail}", log_path.display())
    }
}

#[tauri::command]
pub fn liquid_gravity_probe_detect(
    game_name: String,
    migoto_root: String,
    state: State<'_, LiquidGravityProbeProcess>,
) -> Result<LiquidGravityProbeEditorInfo, String> {
    let (plugin_root, _) = enabled_plugin_context(&game_name)?;
    bundled_editor_root(&plugin_root)?;
    let data_root = probe_data_root(Path::new(&migoto_root));
    let editor_root = data_root.join("Editor");
    let log_path = log_path(&data_root);
    let mut process = state
        .0
        .lock()
        .map_err(|_| "LiquidGravityProbe 状态锁已损坏".to_string())?;
    match refresh_running_state(&mut process) {
        EditorStatus::Healthy => {
            let editor = process.as_ref().expect("running editor");
            Ok(running_info(editor))
        }
        EditorStatus::Unhealthy => {
            let editor = process.as_ref().expect("unhealthy editor");
            let mut info = running_info(editor);
            info.running = false;
            Ok(info)
        }
        EditorStatus::Stopped => Ok(stopped_info(&editor_root, &data_root, &log_path)),
    }
}

#[tauri::command]
pub fn start_liquid_gravity_probe_editor(
    game_name: String,
    migoto_root: String,
    state: State<'_, LiquidGravityProbeProcess>,
) -> Result<LiquidGravityProbeEditorInfo, String> {
    let (plugin_root, python_root) = enabled_plugin_context(&game_name)?;
    let python = discover_python(python_root.as_deref())?;
    let mods_root = mods_root(Path::new(&migoto_root));
    if !mods_root.is_dir() {
        return Err(format!("EFMI Mods 目录不存在：{}", mods_root.display()));
    }
    let data_root = probe_data_root(Path::new(&migoto_root));
    let output_root = data_root.join("output");
    let log_path = log_path(&data_root);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建编辑器日志目录：{error}"))?;
    }
    fs::write(&log_path, "").map_err(|error| format!("无法初始化编辑器日志：{error}"))?;

    let mut process = state
        .0
        .lock()
        .map_err(|_| "LiquidGravityProbe 状态锁已损坏".to_string())?;
    match refresh_running_state(&mut process) {
        EditorStatus::Healthy => {
            let editor = process.as_ref().expect("running editor");
            if editor.plugin_root == plugin_root {
                return Ok(running_info(editor));
            }
            let editor = process.as_mut().expect("running editor");
            stop_child(&mut editor.child)?;
            *process = None;
        }
        EditorStatus::Unhealthy => {
            let editor = process.as_mut().expect("unhealthy editor");
            stop_child(&mut editor.child)?;
            *process = None;
        }
        EditorStatus::Stopped => {}
    }

    let editor_root = materialize_editor_payload(&plugin_root, &data_root)?;
    let port = choose_free_port()?;
    let instance_token = instance_token();
    let mut child = start_external_editor(
        &editor_root,
        &python,
        &mods_root,
        &data_root,
        &output_root,
        port,
        &instance_token,
        &log_path,
    )
    .map_err(|error| startup_error(&error, &log_path))?;

    let client = health_client()?;
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        if child.try_wait().ok().flatten().is_some() {
            return Err(startup_error("Python 编辑器进程提前退出", &log_path));
        }
        if is_editor_healthy(&client, port, &instance_token) {
            let editor = RunningEditor {
                plugin_root: plugin_root.clone(),
                editor_root,
                data_root,
                port,
                instance_token,
                log_path,
                health_misses: 0,
                child,
            };
            let info = running_info(&editor);
            *process = Some(editor);
            return Ok(info);
        }
        thread::sleep(Duration::from_millis(180));
    }
    let _ = stop_child(&mut child);
    Err(startup_error(
        "等待 Liquid Gravity Probe 编辑器启动超时",
        &log_path,
    ))
}

#[tauri::command]
pub fn stop_liquid_gravity_probe_editor(
    state: State<'_, LiquidGravityProbeProcess>,
) -> Result<bool, String> {
    let mut process = state
        .0
        .lock()
        .map_err(|_| "LiquidGravityProbe 状态锁已损坏".to_string())?;
    let Some(mut editor) = process.take() else {
        return Ok(false);
    };
    stop_child(&mut editor.child)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_port_is_available_and_log_tail_is_bounded() {
        let port = choose_free_port().unwrap();
        assert!(port > 0);
        let root = std::env::temp_dir().join(format!(
            "ssmt-liquid-gravity-probe-log-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("editor.log");
        let raw = (0..40)
            .map(|line| format!("line-{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, raw).unwrap();
        let tail = log_tail(&path);
        assert!(tail.starts_with("line-22"));
        assert!(tail.ends_with("line-39"));
        fs::remove_dir_all(root).unwrap();
    }
}
