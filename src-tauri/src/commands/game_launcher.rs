use crate::config::path_manager::PathManager;
use crate::utils::ssmt_file_utils::SSMTFileUtils;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, UpdateKind};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProgramOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

// Removed prepare_game_environment as it's now handled in frontend.

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProgramToLaunch {
    pub path: String,
    pub args: Option<String>,
    pub work_dir: Option<String>,
    pub wait_for_process_name: Option<String>,
    pub wait_timeout_secs: Option<u64>,
    pub wait_only: Option<bool>,
    pub run_as_administrator: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureWwmiLaunchSettingsOptions {
    pub target_exe_path: String,
    #[serde(default = "default_true")]
    pub configure_game: bool,
    #[serde(default)]
    pub apply_perf_tweaks: bool,
    #[serde(default)]
    pub unlock_fps: bool,
    #[serde(default)]
    pub force_max_lod_bias: bool,
    #[serde(default)]
    pub disable_wounded_fx: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureZzmiLaunchSettingsOptions {
    pub target_exe_path: String,
    #[serde(default = "default_true")]
    pub configure_game: bool,
}

fn default_true() -> bool {
    true
}

const ZZMI_GENERAL_DATA_MAGIC: &[u8] = &[
    85, 110, 209, 150, 116, 209, 131, 206, 149, 110, 103, 105, 110, 208, 181, 46, 71, 208, 176,
    109, 101, 206, 159, 98, 106, 101, 209, 129, 116,
];
const SLEEPY_HEADER: &[u8] = &[
    0, 1, 0, 0, 0, 255, 255, 255, 255, 1, 0, 0, 0, 0, 0, 0, 0, 6, 1, 0, 0, 0,
];
const SLEEPY_FOOTER: u8 = 11;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WuwaSettingsRequest {
    target_exe_path: String,
    configure_game: bool,
    apply_perf_tweaks: bool,
    unlock_fps: bool,
    force_max_lod_bias: bool,
    disable_wounded_fx: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WuwaSettingsResponse {
    success: bool,
    message: String,
    error: Option<String>,
}

fn get_program_name_from_path(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn format_program_debug_summary(
    path: &str,
    work_dir: &str,
    args: Option<&str>,
    wait_for_process_name: Option<&str>,
    wait_timeout_secs: Option<u64>,
    wait_only: bool,
) -> String {
    format!(
        "path='{}', name='{}', work_dir='{}', args='{}', wait_for_process='{}', wait_timeout_secs={}, wait_only={}",
        path,
        get_program_name_from_path(path),
        work_dir,
        args.unwrap_or_default(),
        wait_for_process_name.unwrap_or_default(),
        wait_timeout_secs.unwrap_or(0),
        wait_only
    )
}

#[cfg(windows)]
fn run_powershell(script: &str) -> Result<String, String> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|error| format!("Failed to run PowerShell: {}", error))?;

    if !output.status.success() {
        return Err(format!(
            "PowerShell exited with code {:?}. stdout: {} stderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(windows))]
fn run_powershell(_script: &str) -> Result<String, String> {
    Err("Unsupported platform".to_string())
}

#[cfg(windows)]
fn is_elevated() -> Result<bool, String> {
    let script = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; if (([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { '1' } else { '0' }";
    let output = run_powershell(script)?;
    Ok(output.trim() == "1")
}

#[cfg(not(windows))]
fn is_elevated() -> Result<bool, String> {
    Err("Unsupported platform".to_string())
}

fn wuwa_settings_exe_path() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Failed to resolve current exe path: {}", error))?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "Current exe directory is unavailable".to_string())?;
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidates = vec![
        exe_dir.join("wuwa_settings.exe"),
        PathManager::ssmt_resources_folder().join("wuwa_settings.exe"),
        PathManager::ssmt_resources_folder().join("wuwa_settings"),
        current_dir
            .join("src-tauri")
            .join("target")
            .join("debug")
            .join("wuwa_settings.exe"),
        current_dir
            .join("src-tauri")
            .join("target")
            .join("release")
            .join("wuwa_settings.exe"),
        current_dir
            .join("target")
            .join("debug")
            .join("wuwa_settings.exe"),
        current_dir
            .join("target")
            .join("release")
            .join("wuwa_settings.exe"),
    ];

    candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| "wuwa_settings executable was not found".to_string())
}

fn create_wuwa_settings_request_paths() -> Result<(PathBuf, PathBuf), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {}", error))?
        .as_millis();
    let base_dir = std::env::temp_dir().join("ssmt4-wuwa-settings");
    fs::create_dir_all(&base_dir)
        .map_err(|error| format!("Failed to create {}: {}", base_dir.display(), error))?;

    let request_path = base_dir.join(format!(
        "wuwa-settings-{}-{}.json",
        std::process::id(),
        stamp
    ));
    let request_name = request_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Failed to derive wuwa settings request file name".to_string())?;
    let result_file_name = if let Some(stem) = request_name.strip_suffix(".json") {
        format!("{}.result.json", stem)
    } else {
        format!("{}.result.json", request_name)
    };
    let result_path = base_dir.join(result_file_name);
    Ok((request_path, result_path))
}

fn write_wuwa_settings_request(
    request_path: &Path,
    options: &ConfigureWwmiLaunchSettingsOptions,
) -> Result<(), String> {
    let payload = WuwaSettingsRequest {
        target_exe_path: options.target_exe_path.clone(),
        configure_game: options.configure_game,
        apply_perf_tweaks: options.apply_perf_tweaks,
        unlock_fps: options.unlock_fps,
        force_max_lod_bias: options.force_max_lod_bias,
        disable_wounded_fx: options.disable_wounded_fx,
    };
    let raw = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("Failed to serialize wuwa settings request: {}", error))?;
    fs::write(request_path, raw)
        .map_err(|error| format!("Failed to write {}: {}", request_path.display(), error))
}

fn cleanup_wuwa_settings_temp_files(request_path: &Path, result_path: &Path) {
    let _ = fs::remove_file(request_path);
    let _ = fs::remove_file(result_path);
}

fn read_7_bit_encoded_int(data: &[u8], cursor: &mut usize) -> Result<usize, String> {
    let mut result = 0usize;
    for shift in (0..(5 * 7)).step_by(7) {
        let byte = *data
            .get(*cursor)
            .ok_or_else(|| "Unexpected end of Sleepy data length".to_string())?;
        *cursor += 1;

        if shift == 28 && byte > 0b1111 {
            return Err("Invalid Sleepy 7-bit encoded length".to_string());
        }

        result |= usize::from(byte & 0x7f) << shift;
        if byte <= 0x7f {
            return Ok(result);
        }
    }

    Err("Invalid Sleepy 7-bit encoded length".to_string())
}

fn write_7_bit_encoded_int(mut value: usize, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value | 0x80) & 0xff) as u8);
        value >>= 7;
    }
    out.push((value & 0xff) as u8);
}

fn sleepy_evil_map(magic: &[u8]) -> Vec<bool> {
    magic.iter().map(|byte| (*byte & 0xc0) == 0xc0).collect()
}

fn sleepy_decode(data: &[u8], magic: &[u8]) -> Result<String, String> {
    if magic.is_empty() {
        return Err("Sleepy magic cannot be empty".to_string());
    }
    if !data.starts_with(SLEEPY_HEADER) {
        return Err("Invalid Sleepy header in ZZMI GENERAL_DATA.bin".to_string());
    }

    let mut cursor = SLEEPY_HEADER.len();
    let encoded_len = read_7_bit_encoded_int(data, &mut cursor)?;
    let encoded_end = cursor
        .checked_add(encoded_len)
        .ok_or_else(|| "Sleepy encoded length overflow".to_string())?;
    if encoded_end >= data.len() {
        return Err("Sleepy payload is truncated".to_string());
    }
    if data[encoded_end] != SLEEPY_FOOTER {
        return Err("Invalid Sleepy footer in ZZMI GENERAL_DATA.bin".to_string());
    }

    let payload = &data[cursor..encoded_end];
    let evil = sleepy_evil_map(magic);
    let mut eepy = false;
    let mut decoded = Vec::with_capacity(payload.len());

    for (i, encoded_byte) in payload.iter().enumerate() {
        let magic_index = i % magic.len();
        let mut value = *encoded_byte ^ magic[magic_index];
        if evil[magic_index] {
            eepy = value != 0;
            continue;
        }

        if eepy {
            value = value.wrapping_add(0x40);
            eepy = false;
        }
        decoded.push(value);
    }

    String::from_utf8(decoded).map_err(|error| {
        format!(
            "ZZMI GENERAL_DATA.bin decoded to invalid UTF-8 content: {}",
            error
        )
    })
}

fn sleepy_encode(content: &str, magic: &[u8]) -> Result<Vec<u8>, String> {
    if magic.is_empty() {
        return Err("Sleepy magic cannot be empty".to_string());
    }

    let evil = sleepy_evil_map(magic);
    let mut encoded = Vec::with_capacity(content.len() * 2);
    let mut magic_cursor = 0usize;

    for source_byte in content.as_bytes() {
        let mut value = *source_byte;
        let mut magic_index = magic_cursor % magic.len();

        if evil[magic_index] {
            let mut eepy = 0u8;
            if value > 0x40 {
                value -= 0x40;
                eepy = 1;
            }

            encoded.push(eepy ^ magic[magic_index]);
            magic_cursor += 1;
            magic_index = magic_cursor % magic.len();
        }

        encoded.push(value ^ magic[magic_index]);
        magic_cursor += 1;
    }

    let mut out = Vec::with_capacity(SLEEPY_HEADER.len() + encoded.len() + 6);
    out.extend_from_slice(SLEEPY_HEADER);
    write_7_bit_encoded_int(encoded.len(), &mut out);
    out.extend_from_slice(&encoded);
    out.push(SLEEPY_FOOTER);
    Ok(out)
}

fn zzmi_general_data_path(target_exe_path: &str) -> Result<PathBuf, String> {
    let target_path = PathBuf::from(target_exe_path);
    let game_dir = target_path
        .parent()
        .ok_or_else(|| format!("Failed to resolve game directory from {}", target_exe_path))?;

    Ok(game_dir
        .join("ZenlessZoneZero_Data")
        .join("Persistent")
        .join("LocalStorage")
        .join("GENERAL_DATA.bin"))
}

fn new_zzmi_general_data_settings() -> Value {
    let mut settings = Map::new();
    settings.insert(
        "$Type".to_string(),
        Value::String("MoleMole.GeneralLocalDataItem".to_string()),
    );
    settings.insert(
        "userLocalDataVersionId".to_string(),
        Value::String("0.0.1".to_string()),
    );
    Value::Object(settings)
}

fn zzmi_system_settings_map(settings: &mut Value) -> Result<&mut Map<String, Value>, String> {
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "ZZMI GENERAL_DATA.bin root is not a JSON object".to_string())?;

    if !root.contains_key("SystemSettingDataMap") {
        root.insert(
            "SystemSettingDataMap".to_string(),
            Value::Object(Map::new()),
        );
    }

    root.get_mut("SystemSettingDataMap")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            "ZZMI GENERAL_DATA.bin SystemSettingDataMap is not a JSON object".to_string()
        })
}

fn set_zzmi_system_setting(
    settings: &mut Value,
    setting_id: &str,
    new_value: i64,
) -> Result<bool, String> {
    let system_settings = zzmi_system_settings_map(settings)?;

    match system_settings.get_mut(setting_id) {
        None => {
            let mut entry = Map::new();
            entry.insert(
                "$Type".to_string(),
                Value::String("MoleMole.SystemSettingLocalData".to_string()),
            );
            entry.insert("Version".to_string(), Value::Number(0.into()));
            entry.insert("Data".to_string(), Value::Number(new_value.into()));
            system_settings.insert(setting_id.to_string(), Value::Object(entry));
            Ok(true)
        }
        Some(Value::Object(entry)) => {
            let old_value = entry.get("Data").and_then(Value::as_i64).ok_or_else(|| {
                format!(
                    "Unknown ZZMI system setting format for setting {}",
                    setting_id
                )
            })?;
            if old_value == new_value {
                return Ok(false);
            }
            entry.insert("Data".to_string(), Value::Number(new_value.into()));
            Ok(true)
        }
        Some(_) => Err(format!(
            "Unknown ZZMI system setting format for setting {}",
            setting_id
        )),
    }
}

fn apply_zzmi_compatible_settings(settings: &mut Value) -> Result<bool, String> {
    let mut modified = false;
    modified |= set_zzmi_system_setting(settings, "3", 3)?;
    modified |= set_zzmi_system_setting(settings, "13162", 0)?;
    modified |= set_zzmi_system_setting(settings, "99", 1)?;
    Ok(modified)
}

fn serialize_zzmi_settings(settings: &Value) -> Result<String, String> {
    let mut raw = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut raw, formatter);
    settings
        .serialize(&mut serializer)
        .map_err(|error| format!("Failed to serialize ZZMI settings: {}", error))?;
    let json = String::from_utf8(raw)
        .map_err(|error| format!("Failed to encode ZZMI settings as UTF-8: {}", error))?;
    Ok(format!("\r\n{}", json.replace('\n', "\r\n")))
}

fn configure_zzmi_game_settings(target_exe_path: &str) -> Result<(), String> {
    let config_path = zzmi_general_data_path(target_exe_path)?;
    let mut settings = if config_path.exists() {
        let data = fs::read(&config_path)
            .map_err(|error| format!("Failed to read {}: {}", config_path.display(), error))?;
        let raw = sleepy_decode(&data, ZZMI_GENERAL_DATA_MAGIC)?;
        serde_json::from_str(&raw).map_err(|error| {
            format!(
                "Failed to parse ZZMI settings JSON from {}: {}",
                config_path.display(),
                error
            )
        })?
    } else {
        new_zzmi_general_data_settings()
    };

    if !apply_zzmi_compatible_settings(&mut settings)? {
        return Ok(());
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
    }

    let content = serialize_zzmi_settings(&settings)?;
    let encoded = sleepy_encode(&content, ZZMI_GENERAL_DATA_MAGIC)?;
    fs::write(&config_path, encoded)
        .map_err(|error| format!("Failed to write {}: {}", config_path.display(), error))
}

fn run_wuwa_settings_elevated(helper_path: &Path, request_path: &Path) -> Result<(), String> {
    if is_elevated().unwrap_or(false) {
        let status = Command::new(helper_path)
            .arg("--request-json")
            .arg(request_path)
            .status()
            .map_err(|error| format!("Failed to execute {}: {}", helper_path.display(), error))?;
        if status.success() {
            return Ok(());
        }

        return Err(format!(
            "wuwa_settings exited with code {:?}",
            status.code()
        ));
    }

    let escaped_helper = helper_path.to_string_lossy().replace('\'', "''");
    let escaped_work_dir = helper_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .replace('\'', "''");
    let escaped_request = request_path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$p = Start-Process -FilePath '{}' -WorkingDirectory '{}' -ArgumentList @('--request-json','{}') -Verb RunAs -PassThru -Wait; exit $p.ExitCode",
        escaped_helper, escaped_work_dir, escaped_request
    );
    run_powershell(&script)?;
    Ok(())
}

#[tauri::command]
pub async fn configure_zzmi_launch_settings(
    options: ConfigureZzmiLaunchSettingsOptions,
) -> Result<(), String> {
    if !options.configure_game {
        return Ok(());
    }

    configure_zzmi_game_settings(&options.target_exe_path)
}

#[tauri::command]
pub async fn configure_wwmi_launch_settings(
    options: ConfigureWwmiLaunchSettingsOptions,
) -> Result<(), String> {
    let helper_path = wuwa_settings_exe_path()?;
    let (request_path, result_path) = create_wuwa_settings_request_paths()?;
    write_wuwa_settings_request(&request_path, &options)?;

    let run_result = run_wuwa_settings_elevated(&helper_path, &request_path);
    if let Err(error) = run_result {
        cleanup_wuwa_settings_temp_files(&request_path, &result_path);
        return Err(error);
    }

    let result_raw = fs::read_to_string(&result_path).map_err(|error| {
        cleanup_wuwa_settings_temp_files(&request_path, &result_path);
        format!(
            "wuwa_settings did not produce result file {}: {}",
            result_path.display(),
            error
        )
    })?;
    let response: WuwaSettingsResponse = serde_json::from_str(&result_raw).map_err(|error| {
        cleanup_wuwa_settings_temp_files(&request_path, &result_path);
        format!(
            "Failed to parse wuwa_settings result {}: {}",
            result_path.display(),
            error
        )
    })?;

    cleanup_wuwa_settings_temp_files(&request_path, &result_path);

    if response.success {
        Ok(())
    } else {
        Err(response.error.unwrap_or(response.message))
    }
}

#[tauri::command]
pub async fn execute_external_program(
    program_path: String,
    args: Option<Vec<String>>,
    work_dir: Option<String>,
) -> Result<ExternalProgramOutput, String> {
    let mut command = std::process::Command::new(&program_path);

    #[cfg(windows)]
    {
        // 使用 CREATE_NO_WINDOW，避免启动外部程序时弹出控制台窗口。
        command.creation_flags(0x08000000);
    }

    if let Some(args) = args {
        command.args(args);
    }

    let work_dir = work_dir.unwrap_or_else(|| {
        PathBuf::from(&program_path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    });

    let output = command
        .current_dir(work_dir)
        .output()
        .map_err(|e| format!("Failed to execute external program {}: {}", program_path, e))?;

    Ok(ExternalProgramOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[tauri::command]
pub async fn launch_programs(programs: Vec<ProgramToLaunch>) -> Result<(), String> {
    for prog in programs {
        let wait_only = prog.wait_only.unwrap_or(false);
        let args_str = prog.args.as_deref().unwrap_or_default();
        let wait_for_process_name = prog.wait_for_process_name.as_deref();

        if !wait_only {
            let work_dir_str = prog.work_dir.unwrap_or_else(|| {
                let path_obj = PathBuf::from(&prog.path);
                let default_work_dir = PathBuf::from(".");
                path_obj
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(&default_work_dir)
                    .to_string_lossy()
                    .to_string()
            });

            let program_summary = format_program_debug_summary(
                &prog.path,
                &work_dir_str,
                Some(args_str),
                wait_for_process_name,
                prog.wait_timeout_secs,
                wait_only,
            );

            if prog.path.trim().is_empty() {
                let msg = format!("Launch program path is empty. {}", program_summary);
                eprintln!("[GameLauncher] {}", msg);
                return Err(msg);
            }

            let program_path = PathBuf::from(&prog.path);
            if !program_path.exists() {
                let msg = format!("Launch target does not exist. {}", program_summary);
                eprintln!("[GameLauncher] {}", msg);
                return Err(msg);
            }

            if !program_path.is_file() {
                let msg = format!("Launch target is not a file. {}", program_summary);
                eprintln!("[GameLauncher] {}", msg);
                return Err(msg);
            }

            let work_dir_path = PathBuf::from(&work_dir_str);
            if !work_dir_path.exists() {
                let msg = format!(
                    "Launch working directory does not exist. {}",
                    program_summary
                );
                eprintln!("[GameLauncher] {}", msg);
                return Err(msg);
            }

            if !work_dir_path.is_dir() {
                let msg = format!(
                    "Launch working directory is not a directory. {}",
                    program_summary
                );
                eprintln!("[GameLauncher] {}", msg);
                return Err(msg);
            }

            let escaped_path = prog.path.replace('\'', "''");
            let escaped_work_dir = work_dir_str.replace('\'', "''");

            // Use Powershell Start-Process to handle UAC elevation automatically.
            let mut ps_script = format!(
                "try {{ Start-Process -FilePath '{}' -WorkingDirectory '{}'{}",
                escaped_path,
                escaped_work_dir,
                if prog.run_as_administrator.unwrap_or(false) {
                    " -Verb RunAs"
                } else {
                    ""
                }
            );

            if let Some(args) = &prog.args {
                if !args.is_empty() {
                    let escaped_args = args.replace('\'', "''");
                    ps_script.push_str(&format!(" -ArgumentList '{}'", escaped_args));
                }
            }
            ps_script.push_str(
                "; if (-not $?) { exit 1 } } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }",
            );

            println!(
                "[GameLauncher] Launching target via shell: {}",
                program_summary
            );

            let output = std::process::Command::new("powershell")
                .args(&["-NoProfile", "-Command", &ps_script])
                .output()
                .map_err(|e| {
                    let msg = format!(
                        "Failed to launch shell command. {}. os_error={}",
                        program_summary, e
                    );
                    eprintln!("[GameLauncher] {}", msg);
                    msg
                })?;

            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let err_msg = String::from_utf8_lossy(&output.stderr);
                let detailed_error = format!(
                    "Failed to launch shell command. {}. exit_code={:?}. stdout='{}'. stderr='{}'",
                    program_summary,
                    output.status.code(),
                    stdout,
                    err_msg.trim()
                );
                eprintln!("[GameLauncher] {}", detailed_error);
                return Err(detailed_error);
            }
        }

        if let Some(process_name) = prog.wait_for_process_name {
            if !process_name.is_empty() {
                println!("[GameLauncher] Waiting for {} to start...", process_name);
                let mut sys = sysinfo::System::new();
                let start_time = std::time::Instant::now();
                let timeout = std::time::Duration::from_secs(prog.wait_timeout_secs.unwrap_or(30));
                let mut found = false;

                while start_time.elapsed() < timeout {
                    let refresh_kind = ProcessRefreshKind::new().with_exe(UpdateKind::Always);
                    sys.refresh_processes_specifics(refresh_kind);

                    for (_pid, process) in sys.processes() {
                        let proc_name = std::path::Path::new(process.name())
                            .to_string_lossy()
                            .to_lowercase();
                        if proc_name == process_name.to_lowercase() {
                            found = true;
                            break;
                        }
                    }

                    if found {
                        println!("[GameLauncher] {} detected!", process_name);
                        break;
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }

                if !found {
                    println!("[GameLauncher] Timed out waiting for {}.", process_name);
                } else {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn file_md5(path: String) -> Result<String, String> {
    SSMTFileUtils::file_md5(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sleepy_codec_round_trips_ascii_json() {
        let content = "\r\n{\r\n    \"$Type\": \"MoleMole.GeneralLocalDataItem\",\r\n    \"userLocalDataVersionId\": \"0.0.1\"\r\n}";

        let encoded = sleepy_encode(content, ZZMI_GENERAL_DATA_MAGIC).unwrap();
        assert!(encoded.starts_with(SLEEPY_HEADER));

        let decoded = sleepy_decode(&encoded, ZZMI_GENERAL_DATA_MAGIC).unwrap();
        assert_eq!(decoded, content);
    }

    #[test]
    fn sleepy_encode_matches_xxmi_fixture() {
        // Generated from XXMI Launcher's Sleepy.write_string for "\r\n{\"a\":1}".
        let xxmi_fixture = vec![
            0x00, 0x01, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x06, 0x01, 0x00, 0x00, 0x00, 0x0c, 0x58, 0x64, 0xd0, 0xad, 0x56,
            0xd0, 0xa2, 0xce, 0xb7, 0x54, 0x56, 0x14, 0x0b,
        ];

        let encoded = sleepy_encode("\r\n{\"a\":1}", ZZMI_GENERAL_DATA_MAGIC).unwrap();
        assert_eq!(encoded, xxmi_fixture);
        let decoded = sleepy_decode(&xxmi_fixture, ZZMI_GENERAL_DATA_MAGIC).unwrap();
        assert_eq!(decoded, "\r\n{\"a\":1}");
    }

    #[test]
    fn zzmi_settings_are_inserted_and_updated() {
        let mut settings = json!({
            "SystemSettingDataMap": {
                "13162": {
                    "$Type": "MoleMole.SystemSettingLocalData",
                    "Version": 0,
                    "Data": 1
                },
                "99": {
                    "$Type": "MoleMole.SystemSettingLocalData",
                    "Version": 0,
                    "Data": 0
                }
            }
        });

        assert!(apply_zzmi_compatible_settings(&mut settings).unwrap());
        assert_eq!(
            settings["SystemSettingDataMap"]["3"]["Data"],
            Value::from(3)
        );
        assert_eq!(
            settings["SystemSettingDataMap"]["13162"]["Data"],
            Value::from(0)
        );
        assert_eq!(
            settings["SystemSettingDataMap"]["99"]["Data"],
            Value::from(1)
        );

        assert!(!apply_zzmi_compatible_settings(&mut settings).unwrap());
    }

    #[test]
    fn configure_zzmi_game_settings_writes_general_data_file() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let game_dir = std::env::temp_dir().join(format!(
            "ssmt4-zzmi-settings-test-{}-{}",
            std::process::id(),
            stamp
        ));
        let target_exe = game_dir.join("ZenlessZoneZero.exe");

        configure_zzmi_game_settings(&target_exe.to_string_lossy()).unwrap();

        let general_data_path = game_dir
            .join("ZenlessZoneZero_Data")
            .join("Persistent")
            .join("LocalStorage")
            .join("GENERAL_DATA.bin");
        let encoded = fs::read(&general_data_path).unwrap();
        let decoded = sleepy_decode(&encoded, ZZMI_GENERAL_DATA_MAGIC).unwrap();
        let settings: Value = serde_json::from_str(&decoded).unwrap();

        assert_eq!(
            settings["SystemSettingDataMap"]["3"]["Data"],
            Value::from(3)
        );
        assert_eq!(
            settings["SystemSettingDataMap"]["13162"]["Data"],
            Value::from(0)
        );
        assert_eq!(
            settings["SystemSettingDataMap"]["99"]["Data"],
            Value::from(1)
        );

        let _ = fs::remove_dir_all(game_dir);
    }
}
