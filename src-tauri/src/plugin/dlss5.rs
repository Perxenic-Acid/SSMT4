use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DLSS5_PLUGIN_ID: &str = "ssmt.dlss5.integration";
pub const DLSS5_DEPENDENCY_ID: &str = "dlss5-swapper";
pub const DLSS5_BACKUP_DIRECTORY: &str = "_DLSS5_Backup";
pub const DLSS5_MANIFEST_FILE: &str = "manifest.json";
const SUPPORTED_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dlss5Route {
    Native,
    Feeder,
    RenoDx,
    OptiScaler,
    // 空字符串表示 manifest 没有 route；不据此推断 Native 或 Feeder。
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dlss5ManagedState {
    pub route: Dlss5Route,
    pub manifest_path: PathBuf,
    pub game_executable: PathBuf,
    pub game_api: String,
    // Unknown route 的 false 只表示无法由已知 Route 推断，不能作为兼容结论。
    pub uses_reshade: bool,
    // OptiScaler 或 manifest 声明的已知图形代理文件。
    pub uses_proxy: bool,
    // 这些路径始终相对于传入的游戏目录；解析器不会访问它们指向的文件。
    pub added_files: Vec<PathBuf>,
    pub replaced_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dlss5State {
    NotManaged,
    Managed(Dlss5ManagedState),
    Broken {
        manifest_path: PathBuf,
        reason: String,
    },
}

#[derive(Debug, Deserialize)]
struct Dlss5Manifest {
    version: u32,
    #[serde(default)]
    route: Option<String>,
    game: Dlss5GameManifest,
    #[serde(default)]
    added: Vec<String>,
    #[serde(default)]
    replaced: Vec<Dlss5Replacement>,
}

#[derive(Debug, Deserialize)]
struct Dlss5GameManifest {
    exe: String,
    api: String,
}

#[derive(Debug, Deserialize)]
struct Dlss5Replacement {
    rel: String,
}

pub fn inspect_game_directory(game_directory: &Path) -> Dlss5State {
    let manifest_path = game_directory
        .join(DLSS5_BACKUP_DIRECTORY)
        .join(DLSS5_MANIFEST_FILE);
    match inspect(game_directory, &manifest_path) {
        Ok(Some(managed)) => Dlss5State::Managed(managed),
        Ok(None) => Dlss5State::NotManaged,
        Err(reason) => Dlss5State::Broken {
            manifest_path,
            reason,
        },
    }
}

fn inspect(
    game_directory: &Path,
    manifest_path: &Path,
) -> Result<Option<Dlss5ManagedState>, String> {
    let root_metadata = fs::symlink_metadata(game_directory)
        .map_err(|error| format!("cannot inspect game directory: {error}"))?;
    if !root_metadata.is_dir() || is_reparse_point(&root_metadata) {
        return Err("game directory is not a regular directory".to_string());
    }

    let backup_directory = game_directory.join(DLSS5_BACKUP_DIRECTORY);
    let backup_metadata = match fs::symlink_metadata(&backup_directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect DLSS5 backup directory: {error}")),
    };
    if !backup_metadata.is_dir() || is_reparse_point(&backup_metadata) {
        return Err("DLSS5 backup directory is not a regular directory".to_string());
    }

    let manifest_metadata = match fs::symlink_metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect DLSS5 manifest: {error}")),
    };
    if !manifest_metadata.is_file() || is_reparse_point(&manifest_metadata) {
        return Err("DLSS5 manifest is not a regular file".to_string());
    }

    let raw = fs::read_to_string(manifest_path)
        .map_err(|error| format!("cannot read DLSS5 manifest: {error}"))?;
    let manifest: Dlss5Manifest = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid DLSS5 manifest JSON: {error}"))?;
    if manifest.version != SUPPORTED_MANIFEST_VERSION {
        return Err(format!(
            "unsupported DLSS5 manifest version: {}",
            manifest.version
        ));
    }

    let game_executable = validated_relative_path(&manifest.game.exe, "game.exe")?;
    if manifest.game.api.trim().is_empty() {
        return Err("DLSS5 manifest game.api is empty".to_string());
    }
    let added_files = manifest
        .added
        .iter()
        .map(|path| validated_relative_path(path, "added"))
        .collect::<Result<Vec<_>, _>>()?;
    let replaced_files = manifest
        .replaced
        .iter()
        .map(|item| validated_relative_path(&item.rel, "replaced.rel"))
        .collect::<Result<Vec<_>, _>>()?;
    let route = match manifest.route.as_deref() {
        Some(value) if value.eq_ignore_ascii_case("native") => Dlss5Route::Native,
        Some(value) if value.eq_ignore_ascii_case("feeder") => Dlss5Route::Feeder,
        Some(value) if value.eq_ignore_ascii_case("renodx") => Dlss5Route::RenoDx,
        Some(value) if value.eq_ignore_ascii_case("optiscaler") => Dlss5Route::OptiScaler,
        Some(value) => Dlss5Route::Unknown(value.to_string()),
        None => Dlss5Route::Unknown(String::new()),
    };
    let uses_reshade = matches!(
        route,
        Dlss5Route::Native | Dlss5Route::Feeder | Dlss5Route::RenoDx
    );
    let uses_proxy = matches!(route, Dlss5Route::OptiScaler)
        || added_files
            .iter()
            .chain(replaced_files.iter())
            .any(|path| is_graphics_proxy(path));

    Ok(Some(Dlss5ManagedState {
        route,
        manifest_path: manifest_path.to_path_buf(),
        game_executable,
        game_api: manifest.game.api,
        uses_reshade,
        uses_proxy,
        added_files,
        replaced_files,
    }))
}

fn validated_relative_path(raw: &str, field: &str) -> Result<PathBuf, String> {
    super::validate_package_relative_path(raw)
        .map_err(|_| format!("invalid DLSS5 manifest {field} path: {raw}"))?;
    Ok(PathBuf::from(raw.replace('\\', "/")))
}

fn is_graphics_proxy(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            [
                "dxgi.dll",
                "winmm.dll",
                "d3d8.dll",
                "d3d9.dll",
                "ddraw.dll",
                "opengl32.dll",
            ]
            .iter()
            .any(|proxy| name.eq_ignore_ascii_case(proxy))
        })
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct TestGame {
        root: PathBuf,
    }

    impl TestGame {
        fn new() -> Self {
            let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("ssmt-dlss5-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write_manifest(&self, manifest: &Value) {
            let backup = self.root.join(DLSS5_BACKUP_DIRECTORY);
            fs::create_dir_all(&backup).unwrap();
            fs::write(
                backup.join(DLSS5_MANIFEST_FILE),
                serde_json::to_vec(manifest).unwrap(),
            )
            .unwrap();
        }

        fn inspect(&self) -> Dlss5State {
            inspect_game_directory(&self.root)
        }
    }

    impl Drop for TestGame {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn manifest(route: &str) -> Value {
        json!({
            "version": 1,
            "route": route,
            "game": { "dir": "C:\\OldGamePath", "exe": "Game\\Client.exe", "api": "dxgi" },
            "added": ["Game\\dxgi.dll"],
            "replaced": [{ "rel": "Game/ReShade.ini", "kind": "config", "oldVersion": null }],
            "reshade": { "installedByUs": true },
            "futureField": { "ignored": true }
        })
    }

    #[test]
    fn missing_manifest_is_not_managed() {
        let game = TestGame::new();
        assert_eq!(game.inspect(), Dlss5State::NotManaged);
        fs::create_dir_all(game.root.join(DLSS5_BACKUP_DIRECTORY)).unwrap();
        assert_eq!(game.inspect(), Dlss5State::NotManaged);
    }

    #[test]
    fn parses_known_routes_and_owned_files() {
        for (name, expected, uses_reshade) in [
            ("native", Dlss5Route::Native, true),
            ("feeder", Dlss5Route::Feeder, true),
            ("renodx", Dlss5Route::RenoDx, true),
            ("optiscaler", Dlss5Route::OptiScaler, false),
        ] {
            let game = TestGame::new();
            game.write_manifest(&manifest(name));
            let Dlss5State::Managed(state) = game.inspect() else {
                panic!("expected managed state for {name}");
            };
            assert_eq!(state.route, expected);
            assert_eq!(state.game_executable, PathBuf::from("Game/Client.exe"));
            assert_eq!(state.game_api, "dxgi");
            assert_eq!(state.added_files, vec![PathBuf::from("Game/dxgi.dll")]);
            assert_eq!(
                state.replaced_files,
                vec![PathBuf::from("Game/ReShade.ini")]
            );
            assert_eq!(state.uses_reshade, uses_reshade);
            assert!(state.uses_proxy);
        }
    }

    #[test]
    fn unknown_and_missing_routes_are_not_assumed_compatible() {
        let game = TestGame::new();
        game.write_manifest(&manifest("future-route"));
        let Dlss5State::Managed(state) = game.inspect() else {
            panic!("expected managed unknown route");
        };
        assert_eq!(state.route, Dlss5Route::Unknown("future-route".into()));
        assert!(!state.uses_reshade);

        let mut without_route = manifest("native");
        without_route.as_object_mut().unwrap().remove("route");
        game.write_manifest(&without_route);
        let Dlss5State::Managed(state) = game.inspect() else {
            panic!("expected managed route without explicit name");
        };
        assert_eq!(state.route, Dlss5Route::Unknown(String::new()));
    }

    #[test]
    fn route_semantics_do_not_depend_on_an_unrelated_owned_file() {
        let game = TestGame::new();
        let mut value = manifest("optiscaler");
        value["added"] = json!([]);
        value["replaced"] = json!([]);
        game.write_manifest(&value);
        let Dlss5State::Managed(state) = game.inspect() else {
            panic!("expected managed OptiScaler route");
        };
        assert!(!state.uses_reshade);
        assert!(state.uses_proxy);

        value["route"] = json!("feeder");
        game.write_manifest(&value);
        let Dlss5State::Managed(state) = game.inspect() else {
            panic!("expected managed Feeder route");
        };
        assert!(state.uses_reshade);
        assert!(!state.uses_proxy);
    }

    #[test]
    fn malformed_or_unsupported_manifest_is_broken() {
        let game = TestGame::new();
        let backup = game.root.join(DLSS5_BACKUP_DIRECTORY);
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join(DLSS5_MANIFEST_FILE), b"not JSON").unwrap();
        assert!(matches!(game.inspect(), Dlss5State::Broken { .. }));

        let mut unsupported = manifest("native");
        unsupported["version"] = json!(2);
        game.write_manifest(&unsupported);
        assert!(
            matches!(game.inspect(), Dlss5State::Broken { reason, .. } if reason.contains("version"))
        );

        let mut missing_game = manifest("native");
        missing_game.as_object_mut().unwrap().remove("game");
        game.write_manifest(&missing_game);
        assert!(matches!(game.inspect(), Dlss5State::Broken { .. }));

        let mut empty_api = manifest("native");
        empty_api["game"]["api"] = json!(" ");
        game.write_manifest(&empty_api);
        assert!(matches!(game.inspect(), Dlss5State::Broken { .. }));
    }

    #[test]
    fn rejects_unsafe_managed_paths() {
        for (field, path) in [
            ("added", "../outside.dll"),
            ("added", "C:\\Windows\\system32\\dxgi.dll"),
            ("added", "\\\\server\\share\\dxgi.dll"),
            ("replaced", "Game/../../outside.ini"),
            ("game", "..\\outside.exe"),
        ] {
            let game = TestGame::new();
            let mut value = manifest("feeder");
            match field {
                "added" => value["added"] = json!([path]),
                "replaced" => value["replaced"] = json!([{ "rel": path }]),
                "game" => value["game"]["exe"] = json!(path),
                _ => unreachable!(),
            }
            game.write_manifest(&value);
            assert!(
                matches!(game.inspect(), Dlss5State::Broken { .. }),
                "{field}: {path}"
            );
        }
    }

    #[test]
    fn non_file_manifest_is_not_treated_as_absent() {
        let game = TestGame::new();
        fs::create_dir_all(
            game.root
                .join(DLSS5_BACKUP_DIRECTORY)
                .join(DLSS5_MANIFEST_FILE),
        )
        .unwrap();
        assert!(matches!(game.inspect(), Dlss5State::Broken { .. }));
    }

    #[test]
    fn invalid_game_directory_is_broken() {
        let root =
            std::env::temp_dir().join(format!("ssmt-dlss5-missing-game-{}", std::process::id()));
        assert!(matches!(
            inspect_game_directory(&root),
            Dlss5State::Broken { .. }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn symlinked_manifest_is_broken_when_symlinks_are_available() {
        let game = TestGame::new();
        let backup = game.root.join(DLSS5_BACKUP_DIRECTORY);
        fs::create_dir_all(&backup).unwrap();
        let target = game.root.join("target.json");
        fs::write(&target, serde_json::to_vec(&manifest("native")).unwrap()).unwrap();
        let link = backup.join(DLSS5_MANIFEST_FILE);
        if std::os::windows::fs::symlink_file(&target, &link).is_ok() {
            assert!(matches!(game.inspect(), Dlss5State::Broken { .. }));
        }
    }
}
