use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

const SKIPPED_SCAN_DIRS: [&str; 5] = [
    "$recycle.bin",
    "system volume information",
    "windows",
    "recovery",
    "programdata",
];

struct GameDiscoverySpec {
    exe_names: &'static [&'static str],
    desktop_prefixes: &'static [&'static str],
    install_dir_names: &'static [&'static str],
    known_paths: &'static [&'static str],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDiscoveryResult {
    target_exe_path: String,
    launcher_exe_path: String,
}

const GIMI_SPEC: GameDiscoverySpec = GameDiscoverySpec {
    exe_names: &["YuanShen.exe", "GenshinImpact.exe", "Genshin.exe"],
    desktop_prefixes: &["原神", "gi", "genshin", "genshin impact", "yuanshen"],
    install_dir_names: &["Genshin Impact Game"],
    known_paths: &[
        r"C:\Program Files\Genshin Impact bilibili\games\Genshin Impact Game\YuanShen.exe",
        r"C:\Program Files\mihoyo launcher\Genshin Impact\games\Genshin Impact Game\YuanShen.exe",
        r"C:\Program Files\hoyoverse launcher\Genshin Impact\Genshin Impact Game\Genshin.exe",
        r"C:\Program Files\HoYoPlay\games\Genshin Impact Game\GenshinImpact.exe",
    ],
};

const SRMI_SPEC: GameDiscoverySpec = GameDiscoverySpec {
    exe_names: &["StarRail.exe"],
    desktop_prefixes: &[
        "崩坏：星穹铁道",
        "崩坏星穹铁道",
        "星穹铁道",
        "崩铁",
        "sr",
        "starrail",
        "houkai starrail",
        "honkai starrail",
        "hsr",
    ],
    install_dir_names: &["Star Rail Game", "Star Rail Games"],
    known_paths: &[
        r"C:\Program Files\Star Rail bilibili\Games\Star Rail Game\StarRail.exe",
        r"C:\Program Files\mihoyo launcher\Star Rail\games\Star Rail Game\StarRail.exe",
        r"C:\Program Files\HoYoPlay\games\Star Rail Game\StarRail.exe",
        r"C:\Program Files\HoYoPlay\games\Star Rail Games\StarRail.exe",
    ],
};

const ZZMI_SPEC: GameDiscoverySpec = GameDiscoverySpec {
    exe_names: &["ZenlessZoneZero.exe"],
    desktop_prefixes: &["绝区零", "zzz", "zenless", "zenless zone zero"],
    install_dir_names: &["ZenlessZoneZero Game", "Zenless Zone Zero Game"],
    known_paths: &[
        r"C:\Program Files\mihoyo launcher\ZenlessZoneZero\games\ZenlessZoneZero Game\ZenlessZoneZero.exe",
        r"C:\Program Files\HoYoPlay\games\ZenlessZoneZero Game\ZenlessZoneZero.exe",
        r"C:\Program Files\HoYoPlay\games\Zenless Zone Zero Game\ZenlessZoneZero.exe",
    ],
};

fn spec_for_preset(preset: &str) -> Option<&'static GameDiscoverySpec> {
    match preset.trim().to_uppercase().as_str() {
        "GIMI" => Some(&GIMI_SPEC),
        "SRMI" => Some(&SRMI_SPEC),
        "ZZMI" | "ZZMIDX12" => Some(&ZZMI_SPEC),
        _ => None,
    }
}

fn is_game_executable(path: &Path, exe_names: &[&str]) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            exe_names
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

fn first_existing(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_file())
}

fn desktop_candidate(prefixes: &[&str], exe_names: &[&str]) -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE")?;
    let entries = fs::read_dir(PathBuf::from(profile).join("Desktop")).ok()?;
    let mut roots: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            prefixes
                .iter()
                .any(|prefix| name.starts_with(&prefix.to_lowercase()))
                .then(|| entry.path())
        })
        .collect();
    roots.sort();
    roots
        .into_iter()
        .find_map(|root| scan_tree(&root, exe_names, false))
}

#[cfg(windows)]
fn available_drive_roots() -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .filter(|root| root.exists())
        .collect()
}

#[cfg(not(windows))]
fn available_drive_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn scan_tree(root: &Path, exe_names: &[&str], skip_system_dirs: bool) -> Option<PathBuf> {
    let mut pending = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = pending.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_file() && is_game_executable(&path, exe_names) {
                return Some(path);
            }
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            if skip_system_dirs {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if SKIPPED_SCAN_DIRS.contains(&name.as_str()) {
                    continue;
                }
            }
            pending.push_back(path);
        }
    }
    None
}

fn scan_one_directory_level(root: &Path, exe_names: &[&str]) -> Option<PathBuf> {
    let mut directories: Vec<_> = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .collect();
    directories.sort_by_key(|entry| entry.file_name());
    directories
        .into_iter()
        .find_map(|entry| first_existing(exe_names.iter().map(|name| entry.path().join(name))))
}

fn find_game_executable_sync(spec: &GameDiscoverySpec) -> Option<PathBuf> {
    if let Some(path) = first_existing(spec.known_paths.iter().map(PathBuf::from)) {
        return Some(path);
    }
    if let Some(path) = desktop_candidate(spec.desktop_prefixes, spec.exe_names) {
        return Some(path);
    }

    let drives = available_drive_roots();
    for root in &drives {
        for dir_name in spec.install_dir_names {
            let folder = root.join(dir_name);
            if let Some(path) = first_existing(spec.exe_names.iter().map(|name| folder.join(name)))
            {
                return Some(path);
            }
        }
    }
    if let Some(path) = drives
        .iter()
        .find_map(|root| scan_one_directory_level(root, spec.exe_names))
    {
        return Some(path);
    }

    // Last resort: scan every available drive by the preset's exact executable name.
    drives
        .into_iter()
        .find_map(|root| scan_tree(&root, spec.exe_names, true))
}

fn resolve_ntemi_paths(launcher: PathBuf) -> Option<(PathBuf, PathBuf)> {
    let install_dir = launcher.parent()?;
    let expected_target = install_dir
        .join("Client")
        .join("WindowsNoEditor")
        .join("HT")
        .join("Binaries")
        .join("Win64")
        .join("HTGame.exe");
    if expected_target.is_file() {
        return Some((expected_target, launcher));
    }

    // Unknown layouts are searched only inside the launcher installation folder.
    scan_tree(install_dir, &["HTGame.exe"], false).map(|target| (target, launcher))
}

fn find_wwmi_launcher(install_dir: &Path) -> Option<PathBuf> {
    let known_names = [
        "launcher.exe",
        "launcher_epic.exe",
        "Wuthering Waves.exe",
        "WutheringWaves.exe",
        "wuwa.exe",
    ];
    if let Some(path) = first_existing(known_names.map(|name| install_dir.join(name))) {
        return Some(path);
    }

    let mut executables: Vec<_> = fs::read_dir(install_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            name.ends_with(".exe")
                && (name.contains("launcher")
                    || name.contains("wuthering")
                    || name.contains("wuwa"))
        })
        .collect();
    executables.sort_by_key(|entry| entry.file_name());
    executables.first().map(|entry| entry.path())
}

fn resolve_wwmi_paths(install_dir: PathBuf) -> Option<(PathBuf, PathBuf)> {
    let launcher = find_wwmi_launcher(&install_dir)?;
    let relative_targets = [
        r"Wuthering Waves Game\Client\Binaries\Win64\Client-Win64-Shipping.exe",
        r"Wuthering Wave Games\Client\WindowsNoEditor\Binary\Win64\Client-Win64-Shipping.exe",
        r"Wuthering Wave Games\Client\WindowsNoEditor\Binaries\Win64\Client-Win64-Shipping.exe",
    ];
    let target = first_existing(relative_targets.map(|relative| install_dir.join(relative)))
        .or_else(|| scan_tree(&install_dir, &["Client-Win64-Shipping.exe"], false))?;
    Some((target, launcher))
}

fn find_wwmi_executables_sync() -> Option<(PathBuf, PathBuf)> {
    let mut install_roots = vec![PathBuf::from(r"C:\Program Files\Wuthering Waves")];
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let desktop = PathBuf::from(profile).join("Desktop");
        if let Ok(entries) = fs::read_dir(desktop) {
            install_roots.extend(entries.filter_map(Result::ok).filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                (name.starts_with("鸣潮")
                    || name.starts_with("wuwa")
                    || name.starts_with("wuthering"))
                .then(|| entry.path())
            }));
        }
    }
    for drive in available_drive_roots() {
        install_roots.push(drive.join("Wuthering Waves"));
        if let Ok(entries) = fs::read_dir(&drive) {
            install_roots.extend(entries.filter_map(Result::ok).filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                (entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                    && (name.starts_with("wuthering waves") || name.starts_with("鸣潮")))
                .then(|| entry.path())
            }));
        }
    }
    install_roots.sort();
    install_roots.dedup();
    install_roots
        .into_iter()
        .filter(|root| root.is_dir())
        .find_map(resolve_wwmi_paths)
}

fn find_ntemi_executables_sync() -> Option<(PathBuf, PathBuf)> {
    let known_launcher = PathBuf::from(r"D:\Neverness To Everness\NTELauncher.exe");
    if known_launcher.is_file() {
        if let Some(paths) = resolve_ntemi_paths(known_launcher) {
            return Some(paths);
        }
    }

    if let Some(launcher) = desktop_candidate(
        &["异环", "nte", "neverness", "neverness to everness"],
        &["NTELauncher.exe"],
    ) {
        if let Some(paths) = resolve_ntemi_paths(launcher) {
            return Some(paths);
        }
    }

    let drives = available_drive_roots();
    for root in &drives {
        let launcher = root.join("Neverness To Everness").join("NTELauncher.exe");
        if launcher.is_file() {
            if let Some(paths) = resolve_ntemi_paths(launcher) {
                return Some(paths);
            }
        }
    }
    for root in &drives {
        if let Some(launcher) = scan_one_directory_level(root, &["NTELauncher.exe"]) {
            if let Some(paths) = resolve_ntemi_paths(launcher) {
                return Some(paths);
            }
        }
    }

    // Scan for the shallow launcher, never for the deeply nested game executable.
    drives.into_iter().find_map(|root| {
        let launcher = scan_tree(&root, &["NTELauncher.exe"], true)?;
        resolve_ntemi_paths(launcher)
    })
}

#[tauri::command]
pub async fn find_game_executable(
    game_preset: String,
) -> Result<Option<GameDiscoveryResult>, String> {
    let normalized_preset = game_preset.trim().to_uppercase();
    tauri::async_runtime::spawn_blocking(move || {
        if normalized_preset == "NTEMI" {
            return find_ntemi_executables_sync();
        }
        if normalized_preset == "WWMI" {
            return find_wwmi_executables_sync();
        }
        spec_for_preset(&normalized_preset)
            .and_then(find_game_executable_sync)
            .map(|path| (path.clone(), path))
    })
    .await
    .map_err(|error| format!("{game_preset} game discovery task failed: {error}"))
    .map(|paths| {
        paths.map(|(target, launcher)| GameDiscoveryResult {
            target_exe_path: target.to_string_lossy().into_owned(),
            launcher_exe_path: launcher.to_string_lossy().into_owned(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{is_game_executable, spec_for_preset};
    use std::path::Path;

    #[test]
    fn matches_each_supported_preset_executable_case_insensitively() {
        let cases = [
            ("GIMI", "YuanShen.exe"),
            ("GIMI", "GENSHIN.EXE"),
            ("GIMI", "GENSHINIMPACT.EXE"),
            ("SRMI", "STARRAIL.EXE"),
            ("ZZMI", "zenlesszonezero.exe"),
            ("ZZMIDX12", "zenlesszonezero.exe"),
        ];
        for (preset, exe) in cases {
            let spec = spec_for_preset(preset).unwrap();
            assert!(is_game_executable(Path::new(exe), spec.exe_names));
        }
        assert!(!is_game_executable(
            Path::new("launcher.exe"),
            &["StarRail.exe"]
        ));
    }
}
