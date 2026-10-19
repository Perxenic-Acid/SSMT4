use std::fs;
use std::path::{Path, PathBuf};

use crate::common::d3d11_gametype::D3D11GameType;
use crate::config::path_manager::PathManager;

/// 数据类型库根目录：`resources/GameType`，随程序一起发布、一起被覆盖安装与卸载。
///
/// 这是运行时**唯一**的数据类型来源。历史版本还会把它复制一份到用户配置目录，并且只读
/// 用户配置目录那一份，于是卸载残留的旧类型会永远生效、覆盖安装也清不掉；现在类型库只跟
/// 程序走：加类型 = 改仓库里的 `src-tauri/resources/GameType` 并随版本发布。
pub fn bundled_gametype_folder() -> PathBuf {
    PathManager::bundled_gametype_folder()
}

/// 在类型库根目录下定位某个游戏的类型文件夹。
///
/// 目录名匹配不区分大小写（`NarakaM` / `NARAKAM`、`IdentityV` / `IDENTITYV` 都可能出现在
/// 调用方），IDENTITYV 保留指向 `IdentityV` 目录的特例，与 IdentityVNewExtractor 及前端
/// GamePreset 的命名一致。
fn resolve_game_type_folder(root: &Path, game_name: &str) -> Option<PathBuf> {
    let normalized = game_name.trim().to_ascii_uppercase();

    if normalized == "IDENTITYV" {
        return Some(root.join("IdentityV"));
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(folder_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if folder_name.eq_ignore_ascii_case(&normalized) {
                return Some(path);
            }
        }
    }

    let exact = root.join(&normalized);
    exact.is_dir().then_some(exact)
}

/// 从指定 GameType 根目录加载一个游戏的全部 JSON 数据类型。
fn load_game_type_list_from_folder(
    folder: &Path,
    game_name: &str,
) -> Result<Vec<D3D11GameType>, String> {
    if !folder.is_dir() {
        return Err(format!("GameType folder not found: {}", folder.display()));
    }

    let mut json_paths = Vec::new();
    for entry in fs::read_dir(folder)
        .map_err(|e| format!("Failed to read GameType folder {}: {e}", folder.display()))?
    {
        let path = entry
            .map_err(|e| format!("Failed to read GameType entry in {}: {e}", folder.display()))?
            .path();
        let is_json = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if path.is_file() && is_json {
            json_paths.push(path);
        }
    }

    json_paths.sort();

    let mut game_type_list = Vec::new();
    for json_path in json_paths {
        game_type_list.push(D3D11GameType::from_json_file(&json_path)?);
    }

    if game_type_list.is_empty() {
        return Err(format!(
            "No GameType JSON files found in {}",
            folder.display()
        ));
    }

    crate::extract_log!(
        "Loaded {} D3D11GameType entries from JSON files for {}",
        game_type_list.len(),
        game_name
    );
    Ok(game_type_list)
}

/// 读取一个游戏的数据类型列表（JSON 是唯一数据源，类型库随程序发布）。
pub fn get_game_type_list(game_name: &str) -> Result<Vec<D3D11GameType>, String> {
    let normalized_game_name = game_name.trim().to_ascii_uppercase();
    let root = bundled_gametype_folder();
    let Some(folder) = resolve_game_type_folder(&root, &normalized_game_name) else {
        return Err(format!(
            "GameType folder not found for {} under {}",
            normalized_game_name,
            root.display()
        ));
    };
    load_game_type_list_from_folder(&folder, &normalized_game_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled_gametype_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("GameType")
    }

    /// 造一个临时类型库根目录，用来验证目录定位规则。
    fn library_workspace(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "ssmt4-gametype-library-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn all_game_type_folders_load_from_json() {
        let root = bundled_gametype_root();
        let mut loaded_any = false;

        for entry in fs::read_dir(&root)
            .expect("bundled GameType folder should exist")
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let has_json = fs::read_dir(&path)
                .map(|entries| {
                    entries.flatten().any(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext.eq_ignore_ascii_case("json"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if !has_json {
                continue;
            }

            let folder_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let list = load_game_type_list_from_folder(&path, &folder_name)
                .unwrap_or_else(|error| panic!("{folder_name}: {error}"));
            assert!(!list.is_empty(), "{folder_name} should load JSON types");
            loaded_any = true;
        }

        assert!(loaded_any, "no GameType folders found");
    }

    #[test]
    fn identityv_loads_user_added_split2_type() {
        let folder = bundled_gametype_root().join("IdentityV");
        let list = load_game_type_list_from_folder(&folder, "IdentityV")
            .expect("IdentityV JSON should load");
        assert!(
            list.iter()
                .any(|gt| gt.game_type_name == "CPU_P12_N12_TA12_C4_T8_T1-8_Split2"),
            "IdentityV should include the user-added Split2 type"
        );
    }

    #[test]
    fn resolve_game_type_folder_matches_case_insensitively() {
        let root = library_workspace("resolve");

        fs::create_dir_all(root.join("NarakaM")).unwrap();
        assert_eq!(
            resolve_game_type_folder(&root, "narakam"),
            Some(root.join("NarakaM"))
        );

        fs::create_dir_all(root.join("IdentityV")).unwrap();
        assert_eq!(
            resolve_game_type_folder(&root, "IDENTITYV"),
            Some(root.join("IdentityV"))
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_game_type_folder_returns_none_for_unknown_game() {
        let root = library_workspace("missing");

        assert_eq!(resolve_game_type_folder(&root, "NOTAGAME"), None);

        let _ = fs::remove_dir_all(&root);
    }
}
