use super::dlss5::{
    inspect_game_executable, Dlss5ManagedState, Dlss5State, DLSS5_DEPENDENCY_ID, DLSS5_PLUGIN_ID,
};
use super::hoyoshade::{HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};
use super::managed_reshade;
use super::managed_reshade_journal;
use super::managed_cold_files;
use super::registry::PluginRegistry;
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, System, UpdateKind};

pub const DLSS5_ADAPTER_DEPENDENCY_ID: &str = "dlss5-adapter";
pub const DLSS5_PAYLOAD_DEPENDENCY_ID: &str = "dlss5-payload";
static WATCH_GENERATION: AtomicU64 = AtomicU64::new(0);
const SESSION_MARKER: &str = "cleanup-session.txt";

fn marker_path(game_executable: &Path) -> Result<PathBuf, String> {
    let root = game_executable.parent().ok_or("game executable has no parent")?;
    Ok(root.join("_SSMT_Graphics_Backup").join(SESSION_MARKER))
}

fn matches_session(game_executable: &Path, session: &str) -> bool {
    marker_path(game_executable).ok().and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|value| value == session)
}

fn refresh_processes(system: &mut System) {
    system.refresh_processes_specifics(ProcessRefreshKind::new().with_exe(UpdateKind::Always));
}

fn is_target_running(system: &System, target: &Path) -> bool {
    let expected = target.to_string_lossy();
    let expected_name = target.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    system.processes().values().any(|process| {
        process
            .exe()
            .map_or_else(
                || process.name().eq_ignore_ascii_case(expected_name),
                |exe| exe.to_string_lossy().eq_ignore_ascii_case(&expected),
            )
    })
}

fn watch_game_exit(game_executable: PathBuf) -> Result<(), String> {
    let session = format!("{}-{}-{}", std::process::id(),
        WATCH_GENERATION.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?.as_nanos());
    let marker = marker_path(&game_executable)?;
    fs::write(&marker, &session).map_err(|error| format!("cannot record graphics session: {error}"))?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(executable);
    command.arg("--ssmt-graphics-cleanup")
        .arg(game_executable)
        .arg(std::process::id().to_string())
        .arg(&session)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000008);
    }
    command.spawn().map_err(|error| format!("cannot start graphics cleanup helper: {error}"))?;
    Ok(())
}

pub fn cleanup_helper(game_executable: &Path, parent_pid: u32, session: &str) -> Result<(), String> {
    let mut system = System::new();
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut found = false;
    let mut absent_since: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if !matches_session(game_executable, session) { return Ok(()); }
        refresh_processes(&mut system);
        let running = is_target_running(&system, game_executable);
        if running {
            found = true;
            absent_since = None;
        } else if found {
            let since = absent_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(5) { break; }
        } else if system.process(Pid::from_u32(parent_pid)).is_none() || Instant::now() >= deadline {
            break;
        }
    }
    for _ in 0..60 {
        if !matches_session(game_executable, session) { return Ok(()); }
        refresh_processes(&mut system);
        if !is_target_running(&system, game_executable) {
            match restore(game_executable) {
                Ok(_) => return Ok(()),
                Err(error) => {
                    let log = crate::config::path_manager::PathManager::ssmt_global_config_folder()
                        .join("graphics-cleanup.log");
                    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log) {
                        let _ = writeln!(file, "{}: {error}", game_executable.display());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Err(format!("graphics cleanup did not complete for {}", game_executable.display()))
}

pub fn recover_stale_sessions() -> Result<(), String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let mut system = System::new();
    refresh_processes(&mut system);
    let mut errors = Vec::new();
    for executable in registry.managed_game_executables_for_plugin(DLSS5_PLUGIN_ID) {
        if is_target_running(&system, &executable) { continue; }
        let result = match inspect_game_executable(&executable) {
            Dlss5State::Managed(state) => {
                restore(&executable).and_then(|_| managed_cold_files::park_installation(&state))
            }
            Dlss5State::NotManaged => restore(&executable).map(|_| ()),
            Dlss5State::Broken { reason, .. } => Err(reason),
        };
        if let Err(error) = result {
            errors.push(format!("{}: {error}", executable.display()));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedStackStatus {
    pub route: String,
    pub available_routes: Vec<String>,
    pub swapper_owner: String,
    pub hoyoshade_protocol_version: u64,
    pub swapper_protocol_version: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dlss5RouteScan {
    pub routes: Vec<String>,
    pub api: String,
    pub api_label: String,
    pub installed_route: Option<String>,
    pub managed_host: bool,
    pub owner: Option<String>,
    pub anti_cheat_warning: bool,
}

fn dependency_path(
    registry: &PluginRegistry,
    plugin_id: &str,
    dependency_id: &str,
) -> Result<PathBuf, String> {
    let plugin = registry
        .find(plugin_id)
        .ok_or_else(|| format!("plugin not installed: {plugin_id}"))?;
    // DLSS5 使用 HoYoShade 的注入器作为托管 ReShade 宿主；这不等于启用
    // HoYoShade 的效果贡献。仅在本游戏启用 DLSS5 时允许读取该已安装宿主。
    let used_as_dlss5_host = plugin_id == HOYOSHADE_PLUGIN_ID
        && registry.find(DLSS5_PLUGIN_ID).is_some_and(|dlss5| dlss5.enabled);
    if !plugin.enabled && !used_as_dlss5_host {
        return Err(format!("plugin is disabled: {plugin_id}"));
    }
    let state = plugin
        .external_dependencies
        .get(dependency_id)
        .ok_or_else(|| format!("external dependency not declared: {plugin_id}/{dependency_id}"))?;
    if state.status != super::registry::ExternalDependencyStatus::Ready {
        return Err(format!(
            "external dependency is not ready: {plugin_id}/{dependency_id}: {}",
            state.reason.as_deref().unwrap_or("unknown reason")
        ));
    }
    state
        .path
        .clone()
        .ok_or_else(|| format!("external dependency path is missing: {dependency_id}"))
}

fn run_json(program: &Path, arguments: &[&str], timeout: Duration) -> Result<Value, String> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start adapted tool {}: {error}", program.display()))?;
    fn drain(mut pipe: impl Read) -> Result<Vec<u8>, String> {
        let mut output = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = pipe.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                return Ok(output);
            }
            if output.len() + count > 4 * 1024 * 1024 {
                return Err("adapted tool output exceeds 4 MiB".to_string());
            }
            output.extend_from_slice(&buffer[..count]);
        }
    }
    let stdout = child.stdout.take().ok_or("adapted tool has no stdout")?;
    let stderr = child.stderr.take().ok_or("adapted tool has no stderr")?;
    let stdout_reader = std::thread::spawn(move || drain(stdout));
    let stderr_reader = std::thread::spawn(move || drain(stderr));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("adapted tool timed out: {}", program.display()));
            }
            Err(error) => return Err(format!("cannot wait for adapted tool: {error}")),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "adapted tool stdout reader failed")??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "adapted tool stderr reader failed")??;
    let value: Value = serde_json::from_slice(&stdout)
        .map_err(|error| format!("adapted tool returned invalid JSON: {error}"))?;
    if !status.success() || value.get("ok") == Some(&Value::Bool(false)) {
        return Err(format!(
            "adapted tool failed: {}: {} {}",
            value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no detail"),
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    Ok(value)
}

fn check_capabilities(value: &Value, product: &str, flags: &[&str]) -> Result<(), String> {
    if value.get("protocolVersion").and_then(Value::as_u64) != Some(1)
        || value.get("product").and_then(Value::as_str) != Some(product)
        || flags
            .iter()
            .any(|flag| value.get(*flag) != Some(&Value::Bool(true)))
    {
        return Err(format!(
            "{product} adapted interface version or capabilities do not match protocol 1"
        ));
    }
    Ok(())
}

fn check_hoyoshade_adapter(injector: &Path, wait_for_local_dxgi: bool) -> Result<(), String> {
    let capabilities = run_json(injector, &["--ssmt-capabilities"], Duration::from_secs(5))?;
    let mut flags = vec!["managedConfig", "preservesGameAddons"];
    if wait_for_local_dxgi {
        flags.push("waitForLocalDxgi");
    }
    check_capabilities(&capabilities, "HoYoShade", &flags)?;
    let status = run_json(injector, &["--ssmt-status"], Duration::from_secs(5))?;
    if status.get("protocolVersion").and_then(Value::as_u64) != Some(1)
        || status.get("product").and_then(Value::as_str) != Some("HoYoShade")
    {
        return Err("HoYoShade status interface version does not match protocol 1".to_string());
    }
    if status.get("ready").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "HoYoShade adapted installation is incomplete: {}",
            status
                .get("missing")
                .and_then(Value::as_array)
                .map(|missing| missing
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_else(|| "unknown missing resource".to_string())
        ));
    }
    let reshade_version = status
        .get("reshadeVersion")
        .and_then(Value::as_str)
        .ok_or("HoYoShade adapter did not report its ReShade version")?;
    if !supports_dlss5_addon_api(reshade_version) {
        return Err(format!(
            "HoYoShade ReShade {reshade_version} is too old for DLSS5 Add-ons; 6.8.0 or newer is required"
        ));
    }
    Ok(())
}

fn supports_dlss5_addon_api(version: &str) -> bool {
    let parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>();
    match parts {
        Ok(parts) if parts.len() == 4 => (parts[0], parts[1], parts[2]) >= (6, 8, 0),
        _ => false,
    }
}

fn node_program() -> PathBuf {
    PathBuf::from("node")
}

fn adapter_cli(registry: &PluginRegistry) -> Result<PathBuf, String> {
    let root = dependency_path(registry, DLSS5_PLUGIN_ID, DLSS5_ADAPTER_DEPENDENCY_ID)?;
    let cli = root.join("src/ssmt-cli.js");
    if !cli.is_file() {
        return Err(format!("Swapper adapter CLI is missing: {}", cli.display()));
    }
    let cli_arg = cli.to_string_lossy();
    let caps = run_json(
        &node_program(),
        &[&cli_arg, "capabilities"],
        Duration::from_secs(5),
    )?;
    check_capabilities(
        &caps,
        "DLSS5-Swapper",
        &[
            "externalReShadeHost",
            "structuredInstall",
            "structuredRestore",
        ],
    )?;
    Ok(cli)
}

fn game_directory_for(executable: &Path) -> Result<PathBuf, String> {
    if !executable.is_file() {
        return Err(format!(
            "game executable is missing: {}",
            executable.display()
        ));
    }
    match inspect_game_executable(executable) {
        Dlss5State::Managed(state) => state
            .manifest_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or("invalid DLSS5 manifest path".to_string()),
        Dlss5State::Broken { reason, .. } => Err(reason),
        Dlss5State::NotManaged => executable
            .parent()
            .map(Path::to_path_buf)
            .ok_or("game executable has no parent directory".to_string()),
    }
}

fn query_routes_at(
    registry: &PluginRegistry,
    game_dir: &Path,
    executable: &Path,
    api_override: &str,
) -> Result<(Dlss5RouteScan, Value), String> {
    let cli = adapter_cli(registry)?;
    let cli_arg = cli.to_string_lossy();
    let game_dir_arg = game_dir.to_string_lossy();
    let exe_arg = executable.to_string_lossy();
    let override_arg = if api_override == "auto" {
        ""
    } else {
        api_override
    };
    let scan = run_json(
        &node_program(),
        &[&cli_arg, "inspect", &game_dir_arg, &exe_arg, override_arg],
        Duration::from_secs(30),
    )?;
    let routes = scan
        .get("routes")
        .and_then(Value::as_array)
        .ok_or("Swapper did not return routes")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or("invalid Swapper route")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let installed = scan.get("installed").filter(|value| !value.is_null());
    let result = Dlss5RouteScan {
        routes,
        api: scan
            .get("api")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        api_label: scan
            .get("apiLabel")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        installed_route: installed
            .and_then(|value| value.get("route"))
            .and_then(Value::as_str)
            .map(str::to_string),
        managed_host: installed
            .and_then(|value| value.get("managedHost"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        owner: installed
            .and_then(|value| value.get("owner"))
            .and_then(Value::as_str)
            .map(str::to_string),
        anti_cheat_warning: scan
            .get("antiCheatWarning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    Ok((result, scan))
}

fn query_routes(
    registry: &PluginRegistry,
    executable: &Path,
    api_override: &str,
) -> Result<(Dlss5RouteScan, Value), String> {
    let game_dir = game_directory_for(executable)?;
    query_routes_at(registry, &game_dir, executable, api_override)
}

pub fn inspect_routes(
    registry: &PluginRegistry,
    executable: &Path,
    api_override: &str,
) -> Result<Dlss5RouteScan, String> {
    query_routes(registry, executable, api_override).map(|(status, _)| status)
}

fn route_name(state: &Dlss5ManagedState) -> Result<&'static str, String> {
    match state.route {
        super::dlss5::Dlss5Route::Native => Ok("native"),
        super::dlss5::Dlss5Route::Feeder => Ok("feeder"),
        super::dlss5::Dlss5Route::RenoDx => Ok("renodx"),
        super::dlss5::Dlss5Route::OptiScaler => Ok("optiscaler"),
        _ => Err("selected DLSS5 route does not use a managed ReShade host".to_string()),
    }
}

pub fn inspect(
    registry: &PluginRegistry,
    game_executable: &Path,
    state: &Dlss5ManagedState,
) -> Result<ManagedStackStatus, String> {
    let route = route_name(state)?;
    let optiscaler = route == "optiscaler";
    let owner = if optiscaler {
        if state.external_host.is_some() || state.uses_reshade {
            return Err(
                "OptiScaler manifest unexpectedly declares a second ReShade host".to_string(),
            );
        }
        if state
            .added_files
            .iter()
            .chain(&state.replaced_files)
            .any(|file| {
                file.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.eq_ignore_ascii_case("ReShade.ini")
                            || name.eq_ignore_ascii_case("ReShadePreset.ini")
                    })
            })
        {
            return Err("OptiScaler manifest claims SSMT-owned ReShade configuration".to_string());
        }
        "DLSS5-Swapper"
    } else {
        let host = state.external_host.as_ref().ok_or(
            "stock Swapper installation must be restored and reinstalled in SSMT external-host mode",
        )?;
        if host.protocol_version != 1 || host.owner != "SSMT" {
            return Err("DLSS5 manifest does not declare SSMT host ownership".to_string());
        }
        "SSMT"
    };
    let hoyo = dependency_path(registry, HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID)?;
    let swapper = dependency_path(registry, DLSS5_PLUGIN_ID, DLSS5_ADAPTER_DEPENDENCY_ID)?;
    let injector = hoyo.join("inject.exe");
    let cli = swapper.join("src/ssmt-cli.js");
    if !injector.is_file() || !cli.is_file() {
        return Err("adapted injector or Swapper CLI is missing".to_string());
    }
    check_hoyoshade_adapter(&injector, optiscaler)?;
    let cli_path = cli.to_string_lossy();
    let swapper_cap = run_json(
        &node_program(),
        &[&cli_path, "capabilities"],
        Duration::from_secs(5),
    )?;
    check_capabilities(
        &swapper_cap,
        "DLSS5-Swapper",
        &[
            "externalReShadeHost",
            "structuredInstall",
            "structuredRestore",
        ],
    )?;

    let game_dir = state
        .manifest_path
        .parent()
        .and_then(Path::parent)
        .ok_or("invalid DLSS5 manifest location")?;
    let game_dir_arg = game_dir.to_string_lossy();
    let game_exe_arg = game_executable.to_string_lossy();
    let scan = run_json(
        &node_program(),
        &[&cli_path, "inspect", &game_dir_arg, &game_exe_arg],
        Duration::from_secs(30),
    )?;
    let installed = scan
        .get("installed")
        .ok_or("Swapper reports no installation")?;
    if installed.get("route").and_then(Value::as_str) != Some(route)
        || installed.get("managedHost").and_then(Value::as_bool) != Some(!optiscaler)
        || installed.get("owner").and_then(Value::as_str) != Some(owner)
    {
        return Err(
            "Swapper scan and manifest disagree about managed route or ownership".to_string(),
        );
    }
    let available_routes: Vec<String> = scan
        .get("routes")
        .and_then(Value::as_array)
        .ok_or("Swapper scan did not return available routes")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or("invalid route from Swapper")
        })
        .collect::<Result<_, _>>()?;
    if !available_routes.iter().any(|available| available == route) {
        return Err(format!(
            "Swapper no longer offers Route {route} for this game instance"
        ));
    }
    Ok(ManagedStackStatus {
        route: route.to_string(),
        available_routes,
        swapper_owner: owner.to_string(),
        hoyoshade_protocol_version: 1,
        swapper_protocol_version: 1,
    })
}

fn read_config(path: &Path, required: bool) -> Result<String, String> {
    if !path.is_file() {
        return if required {
            Err(format!(
                "required ReShade config is missing: {}",
                path.display()
            ))
        } else {
            Ok(String::new())
        };
    }
    let meta = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 4 * 1024 * 1024 {
        return Err(format!("invalid ReShade config source: {}", path.display()));
    }
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn missing_managed_assets(state: &Dlss5ManagedState) -> Result<Vec<PathBuf>, String> {
    let game_dir = state.manifest_path.parent().and_then(Path::parent)
        .ok_or("invalid DLSS5 manifest location")?;
    let mut missing = Vec::new();
    for relative in state.added_files.iter().chain(&state.replaced_files) {
        let path = game_dir.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Ok(metadata) if metadata.file_type().is_symlink()
                && managed_cold_files::is_parked(state, relative)? => {}
            Ok(_) => return Err(format!("unsafe DLSS5 managed file: {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                && managed_cold_files::is_parked(state, relative)? => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(path),
            Err(error) => return Err(format!("cannot inspect DLSS5 managed file {}: {error}", path.display())),
        }
    }
    Ok(missing)
}

pub fn prepare(
    registry: &PluginRegistry,
    game_executable: &Path,
    state: &Dlss5ManagedState,
    include_hoyoshade_effects: bool,
) -> Result<(), String> {
    inspect(registry, game_executable, state)?;
    let mut system = System::new();
    refresh_processes(&mut system);
    if is_target_running(&system, game_executable) {
        return Err("game is already running; managed configuration cannot be staged".to_string());
    }
    let missing = missing_managed_assets(state)?;
    let repaired_state = if missing.is_empty() {
        None
    } else {
        let names = missing.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ");
        let route = route_name(state)?;
        install_route(registry, game_executable, route, "auto", false)
            .map_err(|error| format!("DLSS5 files are missing ({names}); automatic repair failed: {error}"))?;
        let repaired = match inspect_game_executable(game_executable) {
            Dlss5State::Managed(repaired) if route_name(&repaired)? == route => repaired,
            _ => return Err("DLSS5 repair did not leave a valid managed installation".to_string()),
        };
        let remaining = missing_managed_assets(&repaired)?;
        if !remaining.is_empty() {
            return Err(format!("DLSS5 repair left missing files: {}",
                remaining.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")));
        }
        inspect(registry, game_executable, &repaired)?;
        eprintln!("[GraphicsStack] Repaired missing DLSS5 files for {}: {names}", game_executable.display());
        Some(repaired)
    };
    let state = repaired_state.as_ref().unwrap_or(state);
    let hoyo = dependency_path(registry, HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID)?;
    let base_ini = read_config(&hoyo.join("ReShade.ini"), true)?;
    let base_preset = if include_hoyoshade_effects {
        read_config(&hoyo.join("Presets/Mod OFF.ini"), false)?
    } else {
        String::new()
    };
    let host = state.external_host.as_ref();
    let composed = managed_reshade::compose(
        &base_ini,
        host.map_or("", |host| host.config.game_ini.as_str()),
        &base_preset,
        host.map_or("", |host| host.config.preset.as_str()),
        host.map_or(&[][..], |host| host.config.addon_names.as_slice()),
    )
    .map_err(|error| error.to_string())?;
    let mut addon_sources = Vec::new();
    let hoyo_addons = hoyo.join("reshade-shaders/Addons");
    let addon_dir_meta = fs::symlink_metadata(&hoyo_addons).map_err(|error| error.to_string())?;
    if !addon_dir_meta.is_dir() || addon_dir_meta.file_type().is_symlink() {
        return Err(format!("invalid HoYoShade add-on directory: {}", hoyo_addons.display()));
    }
    if include_hoyoshade_effects {
        for entry in fs::read_dir(&hoyo_addons).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_file() {
                addon_sources.push(path);
            }
        }
    }
    if let Some(host) = host {
        let game_root = game_executable.parent().ok_or("game executable has no parent")?;
        let manifest_root = state.manifest_path.parent().and_then(Path::parent).ok_or("invalid DLSS5 manifest path")?;
        for name in &host.config.addon_names {
            if Path::new(name).file_name().and_then(|part| part.to_str()) != Some(name)
                || name.contains(['/', '\\', ':'])
            {
                return Err(format!("invalid DLSS5 add-on name: {name}"));
            }
            let addon_path = game_root.join(name);
            let relative = addon_path.strip_prefix(manifest_root)
                .map_err(|_| format!("DLSS5 add-on is outside the managed game: {name}"))?;
            addon_sources.push(managed_cold_files::source_path(state, relative)?);
        }
        // Add-ons resolve their runtime and configuration beside the loaded DLL.
        // Link only files that the Swapper manifest claims for this game instance.
        for name in ["nvngx_dlssnr.dll", "nvngx_dlss.dll", "dlss5-feed.cfg"] {
            let path = game_root.join(name);
            if state.added_files.iter().chain(state.replaced_files.iter()).any(|relative| manifest_root.join(relative) == path) {
                let relative = path.strip_prefix(manifest_root).map_err(|error| error.to_string())?;
                addon_sources.push(if managed_cold_files::is_cold_file(relative) {
                    managed_cold_files::source_path(state, relative)?
                } else { path });
            }
        }
    }
    managed_cold_files::park_installation(state)?;
    managed_cold_files::stage(state)?;
    if let Err(error) = managed_reshade_journal::stage(
        game_executable,
        &composed.game_ini,
        &composed.preset_ini,
        &addon_sources,
    ) {
        let recovery = managed_cold_files::park(state);
        return Err(format!("{error}; cold-file recovery: {recovery:?}"));
    }
    if let Err(error) = watch_game_exit(game_executable.to_path_buf()) {
        let _ = restore(game_executable);
        return Err(error);
    }
    Ok(())
}

pub fn restore(game_executable: &Path) -> Result<bool, String> {
    let config = managed_reshade_journal::restore(game_executable);
    let cold = match inspect_game_executable(game_executable) {
        Dlss5State::Managed(state) => managed_cold_files::park(&state),
        _ => Ok(()),
    };
    match (config, cold) {
        (Ok(restored), Ok(())) => {
            let marker = marker_path(game_executable)?;
            match fs::remove_file(&marker) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("cannot retire graphics cleanup session: {error}")),
            }
            let _ = fs::remove_dir(marker.parent().ok_or("invalid graphics cleanup marker")?);
            Ok(restored)
        },
        (Err(config), Ok(())) => Err(config),
        (Ok(_), Err(cold)) => Err(cold),
        (Err(config), Err(cold)) => Err(format!("ReShade restore: {config}; cold-file restore: {cold}")),
    }
}

pub fn restore_swapper(registry: &PluginRegistry, game_executable: &Path) -> Result<(), String> {
    let mut system = System::new();
    refresh_processes(&mut system);
    if is_target_running(&system, game_executable) {
        return Err("close the game before restoring Swapper files".to_string());
    }
    let cli = adapter_cli(registry)?;
    let game_dir = game_directory_for(game_executable)?;
    restore(game_executable)?;
    let previous = match inspect_game_executable(game_executable) {
        Dlss5State::Managed(state) => Some(state),
        _ => None,
    };
    let cli_arg = cli.to_string_lossy();
    let game_dir_arg = game_dir.to_string_lossy();
    run_json(
        &node_program(),
        &[&cli_arg, "restore", &game_dir_arg],
        Duration::from_secs(120),
    )?;
    match inspect_game_executable(game_executable) {
        Dlss5State::NotManaged => {
            if let Some(state) = &previous { managed_cold_files::remove_store(state)?; }
            Ok(())
        }
        Dlss5State::Managed(_) => Err("Swapper reported success but its managed installation remains".to_string()),
        Dlss5State::Broken { reason, .. } => Err(format!("Swapper restore left a broken installation: {reason}")),
    }
}

pub fn open_swapper(registry: &PluginRegistry) -> Result<(), String> {
    let root = dependency_path(registry, DLSS5_PLUGIN_ID, DLSS5_DEPENDENCY_ID)?;
    let executable = root.join("DLSS5-Swapper.exe");
    if !executable.is_file() {
        return Err(format!(
            "Swapper executable is missing: {}",
            executable.display()
        ));
    }
    Command::new(&executable)
        .current_dir(root)
        .spawn()
        .map_err(|error| format!("cannot open DLSS5-Swapper: {error}"))?;
    Ok(())
}

pub fn install_route(
    registry: &PluginRegistry,
    game_executable: &Path,
    route: &str,
    api_override: &str,
    anti_cheat_acknowledged: bool,
) -> Result<Dlss5RouteScan, String> {
    if !["native", "feeder", "renodx"].contains(&route) {
        return Err("select a ReShade route supported by managed mode".to_string());
    }
    let (scan, raw) = query_routes(registry, game_executable, api_override)?;
    let game_dir = game_directory_for(game_executable)?;
    let effective_override = if api_override == "auto" {
        match scan.api_label.as_str() {
            "DirectX 11" => "d3d11",
            "DirectX 12" => "d3d12",
            _ => "auto",
        }
    } else {
        api_override
    };
    if !scan.routes.iter().any(|available| available == route)
        || scan.api != "dxgi"
        || raw.get("bitness").and_then(Value::as_u64) != Some(64)
    {
        return Err(format!(
            "Swapper does not offer managed Route {route} for this game renderer"
        ));
    }
    if raw.get("antiCheatWarning").and_then(Value::as_bool) == Some(true)
        && !anti_cheat_acknowledged
    {
        return Err(
            "Swapper requires explicit anti-cheat risk acknowledgement for this game".to_string(),
        );
    }
    let hoyo = dependency_path(registry, HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID)?;
    check_hoyoshade_adapter(&hoyo.join("inject.exe"), false)?;
    let payload = dependency_path(registry, DLSS5_PLUGIN_ID, DLSS5_PAYLOAD_DEPENDENCY_ID)?;
    let cli = adapter_cli(registry)?;
    let mut system = System::new();
    refresh_processes(&mut system);
    if is_target_running(&system, game_executable) {
        return Err("close the game before installing DLSS5".to_string());
    }
    let old = inspect_game_executable(game_executable);
    if let Dlss5State::Broken { reason, .. } = &old {
        return Err(reason.clone());
    }
    if let Dlss5State::Managed(_) = &old {
        // Swapper 不能更新指向托管存储的链接；先完整恢复旧安装再安装新版本。
        restore_swapper(registry, game_executable)?;
    }
    // Re-scan after a stock restore; route availability can change when its
    // temporary DLLs and proxy are removed.
    let (after_restore, _) =
        query_routes_at(registry, &game_dir, game_executable, effective_override)?;
    if !after_restore
        .routes
        .iter()
        .any(|available| available == route)
    {
        return Err(format!(
            "Route {route} is unavailable after restoring the previous installation"
        ));
    }
    let request = serde_json::json!({
        "gameDir": game_dir,
        "exePath": game_executable,
        "payloadDir": payload,
        "route": route,
        "apiOverride": effective_override,
        "antiCheatAcknowledged": anti_cheat_acknowledged,
        "addMissingDlss": true,
    });
    let unique = WATCH_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let request_file =
        std::env::temp_dir().join(format!("ssmt-dlss5-{}-{unique}.json", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&request_file)
        .map_err(|error| format!("cannot create Swapper request: {error}"))?;
    file.write_all(&serde_json::to_vec(&request).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    let cli_arg = cli.to_string_lossy();
    let request_arg = request_file.to_string_lossy();
    let outcome = run_json(
        &node_program(),
        &[&cli_arg, "install", &request_arg],
        Duration::from_secs(180),
    );
    let _ = fs::remove_file(&request_file);
    outcome?;
    match inspect_game_executable(game_executable) {
        Dlss5State::Managed(installed)
            if installed.external_host.is_some() && route_name(&installed)? == route => {
                if let Err(error) = managed_cold_files::park_installation(&installed) {
                    let rollback = restore_swapper(registry, game_executable);
                    return Err(format!("cannot park DLSS5 cold files: {error}; rollback: {rollback:?}"));
                }
            }
        _ => return Err("Swapper did not leave a valid SSMT-owned manifest".to_string()),
    }
    inspect_routes(registry, game_executable, api_override)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::dlss5::Dlss5Route;
    use serde_json::json;

    #[test]
    fn detects_files_missing_from_managed_installation() {
        let root = std::env::temp_dir().join(format!("ssmt-dlss5-missing-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("_DLSS5_Backup")).unwrap();
        fs::write(root.join("present.addon64"), b"present").unwrap();
        let state = Dlss5ManagedState {
            route: Dlss5Route::Feeder,
            manifest_path: root.join("_DLSS5_Backup/manifest.json"),
            game_executable: PathBuf::from("Game.exe"),
            game_api: "dxgi".into(),
            uses_reshade: true,
            uses_proxy: false,
            added_files: vec![PathBuf::from("present.addon64"), PathBuf::from("missing.addon64")],
            replaced_files: Vec::new(),
            external_host: None,
        };
        assert_eq!(missing_managed_assets(&state).unwrap(), vec![root.join("missing.addon64")]);
        fs::write(root.join("missing.addon64"), b"repaired").unwrap();
        assert!(missing_managed_assets(&state).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn requires_matching_version_and_capabilities() {
        assert!(check_capabilities(
            &json!({"protocolVersion": 1, "product": "HoYoShade",
            "managedConfig": true, "preservesGameAddons": true}),
            "HoYoShade",
            &["managedConfig", "preservesGameAddons"]
        )
        .is_ok());
        assert!(check_capabilities(
            &json!({"protocolVersion": 2, "product": "HoYoShade",
            "managedConfig": true, "preservesGameAddons": true}),
            "HoYoShade",
            &["managedConfig", "preservesGameAddons"]
        )
        .is_err());
    }

    #[test]
    fn requires_reshade_version_with_addon_api_20() {
        assert!(!supports_dlss5_addon_api("6.5.1.2008"));
        assert!(supports_dlss5_addon_api("6.8.0.2155"));
        assert!(!supports_dlss5_addon_api("unknown"));
    }
}
