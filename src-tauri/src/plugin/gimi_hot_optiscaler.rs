use super::*;

/// 汉化 DLL 随已验证的插件包发布；旧包没有该组件时继续使用原固定基线。
/// 不改下载清单，也不把 DLL 放进游戏目录或 Plugin ABI 的自动注入列表。
pub(super) fn stage(
    registry: &PluginRegistry,
    root: &Path,
    profile: &str,
) -> Result<PathBuf, String> {
    let plugin = registry
        .find(DLSS5_PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
        .ok_or("DLSS5 插件未安装或未启用")?;
    let package = canonical_path(&plugin.package_root).map_err(|error| error.to_string())?;
    let dependencies = COMMON
        .iter()
        .filter_map(|(path, hash)| {
            path.strip_prefix("components/OptiScaler/")
                .filter(|name| *name != "OptiScaler.dll")
                .map(|name| (name, *hash))
        })
        .collect::<Vec<_>>();
    stage_at(
        &package,
        root,
        &compatibility_revision(profile)?,
        &dependencies,
    )
}

fn stage_at(
    package: &Path,
    root: &Path,
    baseline: &str,
    dependencies: &[(&str, &str)],
) -> Result<PathBuf, String> {
    match fs::symlink_metadata(package.join("optiscaler")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(root.join("components/OptiScaler"));
        }
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    let source = checked_path(package, "optiscaler/OptiScaler.dll")?;
    let hash = read_text(&checked_path(package, "optiscaler/OptiScaler.dll.sha256")?)?
        .trim()
        .to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("DLSS5 插件的 OptiScaler 构建哈希无效，请重新安装插件包".into());
    }
    verify(&source, &hash)?;

    // 以输入契约而非 DLL 版本命名目录，升级同一兼容基线时保留玩家保存的语言和面板设置。
    let directory = owned_directory(root, &format!("state/optiscaler/{baseline}"))?;
    stage_file(root, &directory, &source, "OptiScaler.dll", &hash)?;
    for (relative, hash) in dependencies {
        let source = checked_path(root, &format!("components/OptiScaler/{relative}"))?;
        verify(&source, hash)?;
        stage_file(root, &directory, &source, relative, hash)?;
    }
    Ok(directory)
}

fn owned_directory(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut directory = root.to_path_buf();
    for component in Path::new(relative).components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("OptiScaler 暂存目录无效".into());
        }
        directory.push(component);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        directory = canonical_path(&directory).map_err(|error| error.to_string())?;
        if !directory.starts_with(root) || !directory.is_dir() {
            return Err("OptiScaler 暂存目录越出热注入资源根目录".into());
        }
    }
    Ok(directory)
}

fn stage_file(
    root: &Path,
    directory: &Path,
    source: &Path,
    relative: &str,
    hash: &str,
) -> Result<(), String> {
    let target = directory.join(relative);
    let parent = target.parent().ok_or("OptiScaler 暂存路径无效")?;
    let relative_parent = parent
        .strip_prefix(root)
        .map_err(|error| error.to_string())?;
    owned_directory(root, &relative_parent.to_string_lossy())?;
    // 现有链接同样必须留在自有目录内，不能因哈希相同就接受外部目标。
    if fs::symlink_metadata(&target).is_ok() {
        checked_path(directory, relative)?;
    }
    if verify(&target, hash).is_err() {
        bridge::copy_atomically(source, &target)?;
    }
    verify(&checked_path(directory, relative)?, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, PathBuf, PluginRegistry) {
        let temp = std::env::temp_dir().join(format!(
            "ssmt-opti-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(temp.join("hot/components/OptiScaler")).unwrap();
        let root = canonical_path(&temp.join("hot")).unwrap();
        let registry = super::super::tests::registry_with_bridge(&temp, b"bridge");
        (temp, root, registry)
    }

    fn write_component(registry: &PluginRegistry, bytes: &[u8]) -> PathBuf {
        let package =
            canonical_path(&registry.find(DLSS5_PLUGIN_ID).unwrap().package_root).unwrap();
        fs::create_dir_all(package.join("optiscaler")).unwrap();
        fs::write(package.join("optiscaler/OptiScaler.dll"), bytes).unwrap();
        fs::write(
            package.join("optiscaler/OptiScaler.dll.sha256"),
            format!("{:x}", Sha256::digest(bytes)),
        )
        .unwrap();
        package
    }

    #[test]
    fn absent_disabled_and_old_plugin_do_not_stage_localized_components() {
        let (temp, root, mut registry) = fixture();
        let empty = PluginRegistry::new(temp.join("empty")).unwrap();
        assert!(stage(&empty, &root, "rtx50").is_err());
        assert_eq!(
            stage(&registry, &root, "rtx50").unwrap(),
            root.join("components/OptiScaler")
        );
        assert!(!root.join("state").exists());
        write_component(&registry, b"localized");
        registry.set_enabled(DLSS5_PLUGIN_ID, false).unwrap();
        assert!(stage(&registry, &root, "rtx50").is_err());
        assert!(!root.join("state").exists());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn upgrades_repair_cached_copies_preserve_settings_and_leave_baseline_intact() {
        let (temp, root, registry) = fixture();
        let source = root.join("components/OptiScaler");
        fs::create_dir_all(source.join("D3D12_Optiscaler")).unwrap();
        fs::write(source.join("OptiScaler.dll"), b"baseline").unwrap();
        fs::write(source.join("nvngx_dlss.dll"), b"dlss").unwrap();
        fs::write(source.join("D3D12_Optiscaler/D3D12Core.dll"), b"agility").unwrap();
        let package = write_component(&registry, b"localized");
        let dlss = format!("{:x}", Sha256::digest(b"dlss"));
        let agility = format!("{:x}", Sha256::digest(b"agility"));
        let dependencies = [
            ("nvngx_dlss.dll", dlss.as_str()),
            ("D3D12_Optiscaler/D3D12Core.dll", agility.as_str()),
        ];
        let staged = stage_at(&package, &root, "fixture-baseline", &dependencies).unwrap();
        assert_eq!(
            fs::read(staged.join("OptiScaler.dll")).unwrap(),
            b"localized"
        );
        assert_eq!(
            fs::read(staged.join("D3D12_Optiscaler/D3D12Core.dll")).unwrap(),
            b"agility"
        );
        let ini = staged.join("OptiScaler.ini");
        fs::write(&ini, "[Menu]\nLanguage=en\nShortcutKey=0x24\n").unwrap();
        fs::write(staged.join("nvngx_dlss.dll"), b"broken").unwrap();
        write_component(&registry, b"localized v2");
        assert_eq!(
            stage_at(&package, &root, "fixture-baseline", &dependencies).unwrap(),
            staged
        );
        assert_eq!(fs::read(staged.join("nvngx_dlss.dll")).unwrap(), b"dlss");
        assert_eq!(
            fs::read(staged.join("OptiScaler.dll")).unwrap(),
            b"localized v2"
        );
        assert!(read_text(&ini).unwrap().contains("Language=en"));
        assert_eq!(
            fs::read(source.join("OptiScaler.dll")).unwrap(),
            b"baseline"
        );
        fs::write(package.join("optiscaler/OptiScaler.dll"), b"corrupt").unwrap();
        assert!(stage_at(&package, &root, "fixture-baseline", &dependencies).is_err());
        fs::write(package.join("optiscaler/OptiScaler.dll.sha256"), "invalid").unwrap();
        assert!(stage_at(&package, &root, "fixture-baseline", &dependencies).is_err());
        fs::remove_file(package.join("optiscaler/OptiScaler.dll")).unwrap();
        assert!(stage_at(&package, &root, "fixture-baseline", &dependencies).is_err());
        fs::remove_dir_all(temp).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn junctions_cannot_stage_outside_the_owned_directory() {
        let (temp, root, registry) = fixture();
        let package = write_component(&registry, b"localized");
        let outside = temp.join("outside");
        fs::create_dir(&outside).unwrap();
        let link = root.join("state");
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:SSMT_TEST_LINK -Target $env:SSMT_TEST_TARGET -ErrorAction Stop | Out-Null"])
            .env("SSMT_TEST_LINK", &link)
            .env("SSMT_TEST_TARGET", &outside)
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stage_at(&package, &root, "fixture-baseline", &[]).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        fs::remove_dir(link).unwrap();
        fs::remove_dir_all(temp).unwrap();
    }
}
