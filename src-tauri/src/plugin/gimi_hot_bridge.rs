use super::*;

/// 仅使用已安装、已启用插件包中的适配组件。完整插件包的发布来源和哈希由
/// 安装器校验；旁置哈希用于发现安装后损坏，不能替代发布来源验证。
pub(super) fn stage(registry: &PluginRegistry, root: &Path) -> Result<PathBuf, String> {
    let plugin = registry
        .find(DLSS5_PLUGIN_ID)
        .filter(|plugin| plugin.enabled)
        .ok_or("DLSS5 插件未安装或未启用")?;
    let package = canonical_path(&plugin.package_root).map_err(|error| error.to_string())?;
    let source = checked_path(&package, "bridge/Dx11FsrBridge.dll").map_err(|_| {
        "请在插件市场更新或重新安装 DLSS5 插件（需要 0.2.1 或更新版本）；已下载的组件无需重新下载".to_string()
    })?;
    let hash_file = checked_path(&package, "bridge/Dx11FsrBridge.dll.sha256")?;
    let hash = read_text(&hash_file)?.trim().to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("DLSS5 插件的 Bridge 构建哈希无效，请重新安装插件包".into());
    }
    verify(&source, &hash)?;
    // 保留固定上游组件及其校验规则。适配组件使用独立副本，升级无需重下 NR。
    let directory = root.join("state/bridge").join(&hash);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let directory = canonical_path(&directory).map_err(|error| error.to_string())?;
    if !directory.starts_with(root) {
        return Err("Bridge 暂存目录越出热注入资源根目录".into());
    }
    let dll = directory.join("Dx11FsrBridge.dll");
    if verify(&dll, &hash).is_err() {
        copy_atomically(&source, &dll)?;
        verify(&dll, &hash)?;
    }
    copy_atomically(
        &checked_path(root, "components/Bridge/Dx11FsrBridge.ini")?,
        &directory.join("Dx11FsrBridge.ini"),
    )?;
    checked_path(&directory, "Dx11FsrBridge.dll")
}

fn copy_atomically(source: &Path, destination: &Path) -> Result<(), String> {
    let temporary = destination.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let result = fs::copy(source, &temporary)
        .and_then(|_| fs::rename(&temporary, destination))
        .map_err(|error| format!("Bridge 组件暂存失败：{error}"));
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
