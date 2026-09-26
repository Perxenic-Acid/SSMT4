use super::dlss5::{
    inspect_game_executable, Dlss5ManagedState, Dlss5State, DLSS5_DEPENDENCY_ID, DLSS5_PLUGIN_ID,
};
use super::hoyoshade::{HOYOSHADE_DEPENDENCY_ID, HOYOSHADE_PLUGIN_ID};
use super::managed_reshade;
use super::managed_reshade_journal;
use super::registry::PluginRegistry;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, System, UpdateKind};

pub const DLSS5_ADAPTER_DEPENDENCY_ID: &str = "dlss5-adapter";
pub const DLSS5_PAYLOAD_DEPENDENCY_ID: &str = "dlss5-payload";
static WATCH_GENERATION: AtomicU64 = AtomicU64::new(0);
static WATCHERS: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();

fn watchers() -> &'static Mutex<HashMap<PathBuf, u64>> {
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn refresh_processes(system: &mut System) {
    system.refresh_processes_specifics(ProcessRefreshKind::new().with_exe(UpdateKind::Always));
}

fn is_target_running(system: &System, target: &Path) -> bool {
    let expected = target.to_string_lossy();
    system.processes().values().any(|process| {
        process
            .exe()
            .is_some_and(|exe| exe.to_string_lossy().eq_ignore_ascii_case(&expected))
    })
}

fn watch_game_exit(game_executable: PathBuf) -> Result<(), String> {
    let generation = WATCH_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    watchers()
        .lock()
        .map_err(|error| error.to_string())?
        .insert(game_executable.clone(), generation);
    std::thread::spawn(move || {
        let mut system = System::new();
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut found = false;
        let mut absent_since: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if watchers()
                .lock()
                .ok()
                .and_then(|entries| entries.get(&game_executable).copied())
                != Some(generation)
            {
                return;
            }
            refresh_processes(&mut system);
            let running = is_target_running(&system, &game_executable);
            if running {
                found = true;
                absent_since = None;
            } else if found {
                let since = absent_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(5) {
                    break;
                }
            } else if Instant::now() >= deadline {
                break;
            }
        }
        if let Err(error) = managed_reshade_journal::restore(&game_executable) {
            eprintln!("[GraphicsStack] Failed to restore config after game exit: {error}");
        }
        if let Ok(mut entries) = watchers().lock() {
            if entries.get(&game_executable) == Some(&generation) {
                entries.remove(&game_executable);
            }
        }
    });
    Ok(())
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
    let hoyo_cap = run_json(&injector, &["--ssmt-capabilities"], Duration::from_secs(5))?;
    check_capabilities(
        &hoyo_cap,
        "HoYoShade",
        &["managedConfig", "preservesGameAddons"],
    )?;
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

pub fn prepare(
    registry: &PluginRegistry,
    game_executable: &Path,
    state: &Dlss5ManagedState,
) -> Result<(), String> {
    inspect(registry, game_executable, state)?;
    let mut system = System::new();
    refresh_processes(&mut system);
    if is_target_running(&system, game_executable) {
        return Err("game is already running; managed configuration cannot be staged".to_string());
    }
    let hoyo = dependency_path(registry, HOYOSHADE_PLUGIN_ID, HOYOSHADE_DEPENDENCY_ID)?;
    let base_ini = read_config(&hoyo.join("ReShade.ini"), true)?;
    let base_preset = read_config(&hoyo.join("Presets/Mod OFF.ini"), false)?;
    let host = state.external_host.as_ref();
    let composed = managed_reshade::compose(
        &base_ini,
        host.map_or("", |host| host.config.game_ini.as_str()),
        &base_preset,
        host.map_or("", |host| host.config.preset.as_str()),
        host.map_or(&[][..], |host| host.config.addon_names.as_slice()),
    )
    .map_err(|error| error.to_string())?;
    managed_reshade_journal::stage(game_executable, &composed.game_ini, &composed.preset_ini)?;
    if let Err(error) = watch_game_exit(game_executable.to_path_buf()) {
        let _ = managed_reshade_journal::restore(game_executable);
        return Err(error);
    }
    Ok(())
}

pub fn restore(game_executable: &Path) -> Result<bool, String> {
    if let Ok(mut entries) = watchers().lock() {
        entries.remove(game_executable);
    }
    managed_reshade_journal::restore(game_executable)
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
    let cli_arg = cli.to_string_lossy();
    let game_dir_arg = game_dir.to_string_lossy();
    run_json(
        &node_program(),
        &[&cli_arg, "restore", &game_dir_arg],
        Duration::from_secs(120),
    )?;
    Ok(())
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
    let caps = run_json(
        &hoyo.join("inject.exe"),
        &["--ssmt-capabilities"],
        Duration::from_secs(5),
    )?;
    check_capabilities(
        &caps,
        "HoYoShade",
        &["managedConfig", "preservesGameAddons"],
    )?;
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
    if let Dlss5State::Managed(old) = &old {
        if old.external_host.is_none() || route_name(old).ok() != Some(route) {
            restore_swapper(registry, game_executable)?;
        } else {
            restore(game_executable)?;
        }
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
            if installed.external_host.is_some() && route_name(&installed)? == route => {}
        _ => return Err("Swapper did not leave a valid SSMT-owned manifest".to_string()),
    }
    inspect_routes(registry, game_executable, api_override)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
