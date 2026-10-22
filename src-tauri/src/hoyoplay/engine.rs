use super::{
    client::{self, Branch, Client, Download, Server},
    manifest::{self, Asset, Chunk},
    space,
    task::Task,
    transaction::{self, Change, Workspace},
    transfer, Context,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub game_name: String,
    pub server_id: String,
    pub operation: String,
    pub directory: String,
    pub audio_languages: Vec<String>,
    #[serde(default)]
    pub confirm_unrecognized_channel: bool,
}
#[derive(Clone)]
struct Resource {
    asset: Asset,
    download: Download,
}
type Resources = BTreeMap<String, Resource>;
#[derive(Default)]
struct Exclusions {
    categories: HashSet<String>,
    files: HashSet<String>,
}

fn exclusions(root: &Path, launch: &serde_json::Value) -> Result<Exclusions, String> {
    let mut excluded = Exclusions::default();
    for (field, enabled) in [
        ("res_category_dir", true),
        ("blacklist_dir", launch["enable_resource_blacklist"] == true),
    ] {
        let Some(name) = launch[field].as_str().filter(|s| enabled && !s.is_empty()) else {
            continue;
        };
        let path = transaction::safe(root, name)?;
        if !path.is_file() {
            continue;
        }
        let text = String::from_utf8(transaction::read_limited(&path, 8 * 1024 * 1024)?)
            .map_err(|e| e.to_string())?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|e| format!("资源裁剪记录损坏：{e}"))?;
            if field == "res_category_dir" {
                if value["is_delete"] == true {
                    let category = value["category"].as_str().ok_or("资源类别记录缺少类别")?;
                    if category != "game" && !client::AUDIO.contains(&category) {
                        excluded.categories.insert(category.into());
                    }
                }
            } else {
                let name = value["fileName"]
                    .as_str()
                    .ok_or("资源黑名单记录缺少文件名")?;
                manifest::relative(name)?;
                excluded.files.insert(key(name));
            }
        }
    }
    Ok(excluded)
}
fn key(name: &str) -> String {
    name.replace('\\', "/").to_lowercase()
}

fn resources(
    http: &Client,
    server: &Server,
    branch: &Branch,
    languages: &[String],
    excluded: &Exclusions,
    manifest_cache: &Path,
    task: &Task,
) -> Result<Resources, String> {
    let mut files: Resources = BTreeMap::new();
    for info in client::build(http, server, branch, languages)? {
        task.check()?;
        if excluded.categories.contains(&info.matching_field) {
            continue;
        }
        for mut asset in manifest::cached(http, &info, manifest_cache)? {
            asset.name = asset.name.replace('\\', "/");
            asset.md5.make_ascii_lowercase();
            for chunk in &mut asset.chunks {
                chunk.md5.make_ascii_lowercase();
                chunk.compressed_md5.make_ascii_lowercase();
            }
            if asset.folder || excluded.files.contains(&key(&asset.name)) {
                continue;
            }
            if let Some(previous) = files.get(&key(&asset.name)) {
                if previous.asset.md5 != asset.md5 || previous.asset.size != asset.size {
                    return Err(format!("资源类别包含互相冲突的文件：{}", asset.name));
                }
            } else {
                files.insert(
                    key(&asset.name),
                    Resource {
                        asset,
                        download: info.chunks.clone(),
                    },
                );
            }
        }
    }
    manifest::validate(&files.values().map(|r| r.asset.clone()).collect::<Vec<_>>())?;
    Ok(files)
}

pub fn ensure_stopped(root: &Path, preset: &str) -> Result<(), String> {
    let targets: Vec<_> = client::servers(preset)
        .iter()
        .map(|s| root.join(&s.exe))
        .collect();
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    for process in system.processes().values() {
        if let Some(exe) = process.exe() {
            if targets.iter().any(|target| {
                target
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&exe.to_string_lossy())
                    || (target.exists()
                        && fs::canonicalize(target).ok() == fs::canonicalize(exe).ok())
            }) {
                return Err("请先关闭这个目录中的游戏，再维护客户端".into());
            }
        }
    }
    Ok(())
}

// 只替换 General 中的安装元信息，其余节、注释和启动器附加字段保留。
pub(super) fn config_ini(original: &str, server: &Server, version: &str) -> String {
    let mut updates = BTreeMap::from([
        ("game_version", version.to_string()),
        ("channel", server.channel.to_string()),
        ("sub_channel", server.sub_channel.to_string()),
        (
            "cps",
            if server.id == "bilibili" {
                "bilibili"
            } else {
                "mihoyo"
            }
            .into(),
        ),
        ("game_biz", server.biz.clone()),
    ]);
    let mut result = vec![];
    let mut general = false;
    let mut found = false;
    for line in original.trim_start_matches('\u{feff}').lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if general {
                result.extend(
                    std::mem::take(&mut updates)
                        .into_iter()
                        .map(|(k, v)| format!("{k}={v}")),
                );
            }
            general = trimmed.eq_ignore_ascii_case("[General]");
            found |= general;
        }
        if general && !trimmed.starts_with([';', '#']) {
            if let Some((k, _)) = trimmed.split_once('=') {
                let lower = k.trim().to_lowercase();
                if let Some(value) = updates.remove(lower.as_str()) {
                    result.push(format!("{}={value}", k.trim()));
                    continue;
                }
            }
        }
        result.push(line.into());
    }
    if !found {
        result.push("[General]".into());
    }
    result.extend(updates.into_iter().map(|(k, v)| format!("{k}={v}")));
    result.join("\r\n") + "\r\n"
}

fn stage_bytes(
    work: &Workspace,
    name: &str,
    bytes: &[u8],
    changes: &mut BTreeMap<String, Change>,
) -> Result<(), String> {
    let target = transaction::safe(&work.root, name)?;
    let before = transaction::hash(&target)?;
    let after = format!("{:x}", md5::compute(bytes));
    if before.as_ref() == Some(&after) {
        changes.remove(&key(name));
        return Ok(());
    }
    let stage = transaction::safe(&work.stage, name)?;
    fs::create_dir_all(stage.parent().ok_or("暂存路径无效")?).map_err(|e| e.to_string())?;
    // 不覆写可能曾用于复用的硬链接。
    if stage.exists() {
        fs::remove_file(&stage).map_err(|e| e.to_string())?;
    }
    let mut file = File::create(stage).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    changes.insert(
        key(name),
        Change {
            name: name.into(),
            before,
            after: Some(after),
        },
    );
    Ok(())
}

fn channel_sdk(
    http: &Client,
    server: &Server,
    work: &Workspace,
    changes: &mut BTreeMap<String, Change>,
    task: &Task,
    stage: bool,
) -> Result<BTreeMap<String, String>, String> {
    let Some(sdk) = client::sdk(http, server)? else {
        if server.id == "bilibili" && server.biz == "hk4e_cn" {
            return Err("官方未提供原神 B服渠道 SDK，暂不提交客户端".into());
        }
        return Ok(BTreeMap::new());
    };
    let package = &sdk["channel_sdk_pkg"];
    let size = client::number(package, "size")?;
    if size > 128 * 1024 * 1024 {
        return Err("渠道 SDK 超出支持大小".into());
    }
    let checksum = package["md5"].as_str().ok_or("渠道 SDK 校验值缺失")?;
    client::validate_md5(checksum)?;
    task.check()?;
    let cache = transaction::safe(&work.work, &format!("sdk-{}.zip", checksum.to_lowercase()))?;
    let bytes = if transaction::hash(&cache)?
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case(checksum))
    {
        fs::read(&cache).map_err(|e| e.to_string())?
    } else {
        client::get_bytes(
            http,
            package["url"].as_str().ok_or("渠道 SDK 下载地址缺失")?,
            size,
        )?
    };
    if bytes.len() as u64 != size
        || !format!("{:x}", md5::compute(&bytes)).eq_ignore_ascii_case(checksum)
    {
        return Err("渠道 SDK 下载校验失败".into());
    }
    fs::write(cache, &bytes).map_err(|e| e.to_string())?;
    sdk_entries(work, &bytes, changes, stage)
}

pub(super) fn sdk_entries(
    work: &Workspace,
    bytes: &[u8],
    changes: &mut BTreeMap<String, Change>,
    stage: bool,
) -> Result<BTreeMap<String, String>, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut names = std::collections::HashSet::new();
    let mut total = 0u64;
    let mut files = BTreeMap::new();
    if archive.len() > 20000 {
        return Err("渠道 SDK 文件过多".into());
    }
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        manifest::relative(name.trim_end_matches('/'))?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) || !names.insert(key(&name))
        {
            return Err("渠道 SDK 包含链接或重复路径".into());
        }
        if entry.is_dir() {
            continue;
        }
        total = total.checked_add(entry.size()).ok_or("SDK 大小溢出")?;
        if total > 512 * 1024 * 1024 || entry.size() > 128 * 1024 * 1024 {
            return Err("渠道 SDK 解压大小超出限制".into());
        }
        let mut content = Vec::new();
        let size = entry.size();
        (&mut entry)
            .take(size + 1)
            .read_to_end(&mut content)
            .map_err(|e| e.to_string())?;
        if content.len() as u64 != size {
            return Err("渠道 SDK 文件长度不符".into());
        }
        files.insert(name.clone(), format!("{:x}", md5::compute(&content)));
        if stage {
            stage_bytes(work, &name, &content, changes)?;
        }
    }
    Ok(files)
}

fn reuse_name(name: &str, preset: &str, from: Option<&str>, to: &str) -> String {
    if preset != "GIMI" {
        return name.into();
    }
    match (from == Some("global"), to == "global") {
        (true, false) => name.replacen("YuanShen_Data/", "GenshinImpact_Data/", 1),
        (false, true) => name.replacen("GenshinImpact_Data/", "YuanShen_Data/", 1),
        _ => name.into(),
    }
}

fn assemble(
    resource: &Resource,
    old: Option<&Resource>,
    work: &Workspace,
    cache: &Path,
    source_name: &str,
    http: &Client,
    task: &Task,
    streaming: Option<&transfer::StreamingCache>,
) -> Result<(), String> {
    let asset = &resource.asset;
    let source = transaction::safe(&work.root, source_name)?;
    let mut previous = File::open(&source).ok();
    let offsets: HashMap<_, _> = old
        .into_iter()
        .flat_map(|r| &r.asset.chunks)
        .map(|c| ((c.md5.as_str(), c.size), c.offset))
        .collect();
    let stage = transaction::safe(&work.stage, &asset.name)?;
    let mut output = if streaming.is_none() {
        None
    } else {
        fs::create_dir_all(stage.parent().ok_or("暂存路径无效")?).map_err(|e| e.to_string())?;
        Some(
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&stage)
                .map_err(|e| e.to_string())?,
        )
    };
    let staged_size = output
        .as_ref()
        .map(|f| f.metadata().map(|m| m.len()))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    for chunk in &asset.chunks {
        task.check()?;
        // 中断后先校验暂存块，绝不截断重建已经解压的数据。压缩缓存释放后仍可续装。
        let mut reused = output
            .as_mut()
            .filter(|_| staged_size >= chunk.offset + chunk.size)
            .and_then(|file| reuse_chunk(file, chunk.offset, chunk));
        let staged = reused.is_some();
        if reused.is_none() {
            if let Some(file) = &mut previous {
                let offset = offsets
                    .get(&(chunk.md5.as_str(), chunk.size))
                    .copied()
                    .unwrap_or(chunk.offset);
                reused = reuse_chunk(file, offset, chunk);
            }
        }
        let bytes = match reused {
            Some(bytes) => bytes,
            None => transfer::chunk(http, &resource.download, chunk, cache, task)?,
        };
        if let Some(output) = &mut output {
            if !staged {
                output
                    .seek(SeekFrom::Start(chunk.offset))
                    .and_then(|_| output.write_all(&bytes))
                    .map_err(|e| e.to_string())?;
            }
        }
        if let Some(streaming) = streaming {
            streaming.consumed(cache, chunk, task)?;
        }
        task.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bytes_done += chunk.size;
    }
    if let Some(file) = output {
        file.set_len(asset.size).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        if transaction::hash(&stage)?.as_ref() != Some(&asset.md5) {
            return Err(format!("完整文件校验失败：{}", asset.name));
        }
    }
    Ok(())
}

fn reuse_chunk(file: &mut File, offset: u64, chunk: &Chunk) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; chunk.size as usize];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut bytes))
        .ok()?;
    (format!("{:x}", md5::compute(&bytes)) == chunk.md5).then_some(bytes)
}

pub fn run(request: Request, task: &Task) -> Result<(), String> {
    if !["install", "update", "preload", "switch", "recover"].contains(&request.operation.as_str())
    {
        return Err("未知维护操作".into());
    }
    let context = Context::load(&request.game_name)?;
    let root = PathBuf::from(&request.directory);
    if !["install", "recover"].contains(&request.operation.as_str())
        && context.root.as_ref().and_then(|r| fs::canonicalize(r).ok())
            != fs::canonicalize(&root).ok()
    {
        return Err("维护目录与游戏实例目标路径不一致".into());
    }
    if request.operation != "preload" {
        ensure_stopped(&root, &context.preset)?;
    }
    let work = Workspace::open(&root)?;
    work.cleanup_completed()?;
    if request.operation == "recover" {
        task.phase("commit", "恢复中断的客户端文件事务");
        return work.recover(&context.config_path);
    }
    if work.work.join("journal.json").exists() {
        return Err("上次文件提交中断，请先点击恢复原客户端".into());
    }
    if request.audio_languages.is_empty()
        || request
            .audio_languages
            .iter()
            .any(|l| !client::AUDIO.contains(&l.as_str()))
    {
        return Err("请至少选择一种受支持的语音".into());
    }
    let servers = client::servers(&context.preset);
    let server = servers
        .iter()
        .find(|s| s.id == request.server_id)
        .ok_or("未知游戏渠道")?;
    let mut current = super::detect_server(&context.preset, &work.root)?;
    if current.is_none()
        && request.confirm_unrecognized_channel
        && ["update", "preload"].contains(&request.operation.as_str())
        && work.root.join(&server.exe).is_file()
    {
        current = Some(server.id.clone());
    }
    if request.operation == "install" {
        if current.is_some() {
            return Err("目录已有游戏客户端，请使用更新 / 修复".into());
        }
        if fs::read_dir(&work.root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .any(|e| e.file_name() != ".ssmt-maintenance")
        {
            return Err("首次安装请选择空目录，以免覆盖其他文件".into());
        }
    } else if current.is_none() {
        return Err("无法识别当前客户端渠道，请先通过官方启动器修复 config.ini".into());
    }
    if request.operation == "switch" {
        if context.preset != "GIMI" {
            return Err("首批只支持原神客户端换服".into());
        }
        if current.as_deref() == Some(&server.id) {
            return Err("当前已是所选渠道".into());
        }
    } else if current.as_deref().is_some_and(|s| s != server.id) {
        return Err("更新及预下载不能更改渠道；原神请使用换服按钮".into());
    }
    let local_ini = super::ini_values(&work.root)?;
    let local_version = local_ini.get("game_version");
    let http = client::http()?;
    let launch = client::launch_config(&http, server)?;
    let mut languages = request.audio_languages.clone();
    if let Some(current) = &current {
        let source_server = servers
            .iter()
            .find(|s| s.id == *current)
            .ok_or("原渠道未知")?;
        let source_launch = if current == &server.id {
            launch.clone()
        } else {
            client::launch_config(&http, source_server)?
        };
        languages = super::installed_audio(&work.root, &source_launch, &languages)?;
    }
    let (main, pre) = client::branches(&http, server)?;
    let branch = if request.operation == "preload" {
        pre.ok_or("官方尚未开放预下载")?
    } else {
        main
    };
    if let Some(local) = local_version {
        if client::version(local)? > client::version(&branch.tag)? {
            return Err("本地版本高于所选官方分支，拒绝降级".into());
        }
    }
    if request.operation == "switch" && local_version != Some(&branch.tag) {
        return Err("请先更新到与目标渠道相同的当前版本，再换服".into());
    }
    task.snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .target_version = branch.tag.clone();
    let preload = request.operation == "preload";
    task.phase(
        if preload { "preload" } else { "manifest" },
        "读取并校验官方分块清单",
    );
    let excluded = if request.operation == "install" {
        Exclusions::default()
    } else {
        exclusions(&work.root, &launch)?
    };
    let manifest_cache = transaction::safe(&work.work, "manifests")?;
    fs::create_dir_all(&manifest_cache).map_err(|e| e.to_string())?;
    let files = resources(
        &http,
        server,
        &branch,
        &languages,
        &excluded,
        &manifest_cache,
        task,
    )?;
    if !files.contains_key(&key(&server.exe)) {
        return Err("官方清单缺少目标游戏程序".into());
    }
    let mut old_files = BTreeMap::new();
    if let (Some(id), Some(version)) = (&current, local_version) {
        let old_server = servers.iter().find(|s| s.id == *id).ok_or("原渠道未知")?;
        let old_result = (|| {
            if id == &server.id && version == &branch.tag {
                return Ok(files.clone());
            }
            let (mut old_branch, _) = client::branches(&http, old_server)?;
            old_branch.tag = version.clone();
            resources(
                &http,
                old_server,
                &old_branch,
                &languages,
                &excluded,
                &manifest_cache,
                task,
            )
        })();
        match old_result {
            Ok(files) => old_files = files,
            Err(e) if request.operation == "switch" => {
                return Err(format!("无法验证原渠道文件，取消换服：{e}"))
            }
            Err(_) => {} // 旧清单失效时仅逐块校验复用，不清除未知旧文件。
        }
    }
    let cache = transaction::safe(&work.work, "cache")?;
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    task.phase(
        if preload { "preload" } else { "verify" },
        "校验现有文件和可恢复缓存",
    );
    let mut changes = BTreeMap::new();
    let mut pending = vec![];
    let mut stage_bytes_needed = 0u64;
    let mut missing_cache = HashMap::new();
    for (index, resource) in files.values().enumerate() {
        task.check()?;
        task.phase(
            if preload { "preload" } else { "verify" },
            format!(
                "检查资源 {}/{}：{}",
                index + 1,
                files.len(),
                resource.asset.name
            ),
        );
        let asset = &resource.asset;
        let before = checked_hash(&transaction::safe(&work.root, &asset.name)?, task)?;
        if before.as_ref() == Some(&asset.md5) {
            continue;
        }
        let stage = transaction::safe(&work.stage, &asset.name)?;
        let ready = !preload && checked_hash(&stage, task)?.as_ref() == Some(&asset.md5);
        if !preload {
            changes.insert(
                key(&asset.name),
                Change {
                    name: asset.name.clone(),
                    before,
                    after: Some(asset.md5.clone()),
                },
            );
        }
        if ready {
            continue;
        }
        if !preload {
            // 已有暂存数据会逐块重验并原位写回；其当前长度无需再次申请磁盘空间。
            let present = space::staged_credit(&stage, asset.size)?;
            stage_bytes_needed = space::sum([stage_bytes_needed, asset.size - present])?;
        }
        for chunk in &asset.chunks {
            if preload {
                let path = transfer::cache_path(&cache, chunk)?;
                if !path.exists() {
                    missing_cache.insert(path, chunk.compressed_size);
                }
            }
        }
        pending.push(resource);
    }
    let needed = if preload {
        space::sum([space::sum(missing_cache.into_values())?, space::RESERVE])?
    } else {
        space::streaming_required(stage_bytes_needed)?
    };
    space::check(&work.root, needed)?;
    let streaming = if preload {
        None
    } else {
        Some(transfer::StreamingCache::new(
            &cache,
            pending.iter().flat_map(|r| &r.asset.chunks),
        )?)
    };
    {
        let mut s = task.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        s.bytes_total = pending.iter().map(|r| r.asset.size).sum();
        s.files_total = pending.len() as u64;
    }
    task.phase(
        if preload { "preload" } else { "download" },
        "复用已校验分块并下载缺失数据",
    );
    let next = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        for _ in 0..space::WORKERS {
            scope.spawn(|| loop {
                if task.check().is_err() || !errors.lock().unwrap().is_empty() {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(resource) = pending.get(index) else {
                    break;
                };
                let source = reuse_name(
                    &resource.asset.name,
                    &context.preset,
                    current.as_deref(),
                    &server.id,
                );
                let old = old_files.get(&key(&source));
                match assemble(
                    resource,
                    old,
                    &work,
                    &cache,
                    &source,
                    &http,
                    task,
                    streaming.as_ref(),
                ) {
                    Ok(()) => {
                        task.snapshot
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .files_done += 1
                    }
                    Err(e) => {
                        errors.lock().unwrap().push(e);
                        break;
                    }
                }
            });
        }
    });
    if let Some(error) = errors.into_inner().unwrap().into_iter().next() {
        return Err(error);
    }
    task.check()?;
    if preload {
        return Ok(());
    }
    task.phase("sdk", "部署所选渠道 SDK");
    let sdk = channel_sdk(&http, server, &work, &mut changes, task, true)?;
    if request.operation == "switch" {
        let old_server = servers
            .iter()
            .find(|s| Some(s.id.as_str()) == current.as_deref())
            .ok_or("原渠道未知")?;
        let old_sdk = channel_sdk(&http, old_server, &work, &mut changes, task, false)?;
        for (name, checksum) in old_sdk {
            if files.contains_key(&key(&name))
                || sdk.keys().any(|n| key(n) == key(&name))
                || changes.contains_key(&key(&name))
            {
                continue;
            }
            let before = transaction::hash(&transaction::safe(&work.root, &name)?)?;
            if before.as_ref() == Some(&checksum) {
                changes.insert(
                    key(&name),
                    Change {
                        name,
                        before,
                        after: None,
                    },
                );
            }
        }
    }
    // 只移除清单证明归属且未被修改的过期资源，不扫描或递归清理 Mod 目录。
    for (name, old) in &old_files {
        if files.contains_key(name) || changes.contains_key(name) || name == "config.ini" {
            continue;
        }
        let before = transaction::hash(&transaction::safe(&work.root, &old.asset.name)?)?;
        if before.as_ref() == Some(&old.asset.md5) {
            changes.insert(
                name.clone(),
                Change {
                    name: old.asset.name.clone(),
                    before,
                    after: None,
                },
            );
        }
    }
    if request.operation == "switch" {
        let previous_exe = &servers
            .iter()
            .find(|s| Some(s.id.as_str()) == current.as_deref())
            .ok_or("原渠道未知")?
            .exe;
        if previous_exe != &server.exe
            && !changes
                .get(&key(previous_exe))
                .is_some_and(|c| c.after.is_none())
        {
            return Err("原客户端程序已被修改，无法安全移除，取消换服".into());
        }
    }
    let ini = transaction::safe(&work.root, "config.ini")?;
    let text = if ini.exists() {
        fs::read_to_string(ini).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    stage_bytes(
        &work,
        "config.ini",
        config_ini(&text, server, &branch.tag).as_bytes(),
        &mut changes,
    )?;
    if let Some(name) = launch["audio_pkg_scan_dir"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        let names: Vec<_> = languages
            .iter()
            .filter_map(|s| match s.as_str() {
                "zh-cn" => Some("Chinese"),
                "en-us" => Some("English(US)"),
                "ja-jp" => Some("Japanese"),
                "ko-kr" => Some("Korean"),
                _ => None,
            })
            .collect();
        stage_bytes(
            &work,
            name,
            (names.join("\n") + "\n").as_bytes(),
            &mut changes,
        )?;
    }
    let before = json!({ "targetExePath":context.config["targetExePath"], "hoyoplayServer":context.config["hoyoplayServer"], "hoyoplayAudioLanguages":context.config["hoyoplayAudioLanguages"] });
    let after = json!({ "targetExePath":root.join(&server.exe), "hoyoplayServer":server.id, "hoyoplayAudioLanguages":languages });
    task.check()?;
    ensure_stopped(&work.root, &context.preset)?;
    task.phase("commit", "提交已校验文件，请勿启动游戏或关闭 SSMT");
    let mut changes: Vec<_> = changes.into_values().collect();
    changes.sort_by_key(|c| c.name.eq_ignore_ascii_case("config.ini"));
    work.commit(changes, &context.config_path, before, after)?;
    if let Err(e) = work.cleanup_completed() {
        eprintln!("[HoYoPlay] 客户端已完成维护，备份清理留待下次：{e}");
    }
    // 成功后仅清理本清单中的已验证缓存，预下载分支的其他缓存继续保留。
    for resource in files.values() {
        for chunk in &resource.asset.chunks {
            if let Ok(path) = transfer::cache_path(&cache, chunk) {
                if path.exists() {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
    Ok(())
}

fn checked_hash(path: &Path, task: &Task) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        task.check()?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        context.consume(&buffer[..count]);
    }
    Ok(Some(format!("{:x}", context.compute())))
}

#[cfg(test)]
mod tests {
    use super::super::manifest::Chunk;
    use super::super::tests::Temp;
    use super::*;
    fn resource(name: &str, parts: &[&[u8]]) -> Resource {
        let mut offset = 0;
        let chunks = parts
            .iter()
            .enumerate()
            .map(|(i, bytes)| {
                let chunk = Chunk {
                    id: format!("fixture-{i}"),
                    offset,
                    size: bytes.len() as u64,
                    compressed_size: bytes.len() as u64,
                    md5: format!("{:x}", md5::compute(bytes)),
                    compressed_md5: format!("{:x}", md5::compute(bytes)),
                };
                offset += bytes.len() as u64;
                chunk
            })
            .collect();
        Resource {
            asset: Asset {
                name: name.into(),
                md5: format!("{:x}", md5::compute(parts.concat())),
                chunks,
                folder: false,
                size: offset,
            },
            download: Download {
                url_prefix: "unsupported://must-not-be-requested".into(),
                url_suffix: String::new(),
                compression: 0,
                encryption: 0,
            },
        }
    }
    #[test]
    fn shifted_chunks_are_reused_and_cross_region_source_stays_intact() {
        let temp = Temp::new();
        let work = Workspace::open(&temp.0.join("game")).unwrap();
        fs::create_dir_all(work.root.join("YuanShen_Data")).unwrap();
        fs::write(work.root.join("YuanShen_Data/model"), b"ABCD").unwrap();
        let old = resource("YuanShen_Data/model", &[b"AB", b"CD"]);
        let new = resource("GenshinImpact_Data/model", &[b"CD", b"AB"]);
        let task = Task::new("fixture".into(), String::new());
        let cache = work.work.join("cache");
        fs::create_dir(&cache).unwrap();
        let source = reuse_name(&new.asset.name, "GIMI", Some("cn"), "global");
        assemble(
            &new,
            Some(&old),
            &work,
            &cache,
            &source,
            &client::http().unwrap(),
            &task,
            Some(&transfer::StreamingCache::new(&cache, new.asset.chunks.iter()).unwrap()),
        )
        .unwrap();
        assert_eq!(
            fs::read(work.stage.join("GenshinImpact_Data/model")).unwrap(),
            b"CDAB"
        );
        assert_eq!(
            fs::read(work.root.join("YuanShen_Data/model")).unwrap(),
            b"ABCD"
        );
        assert!(!work.root.join("GenshinImpact_Data/model").exists());
    }
    #[test]
    fn preload_cache_is_consumed_by_update_without_writing_the_live_client() {
        let temp = Temp::new();
        let work = Workspace::open(&temp.0.join("game")).unwrap();
        let cache = work.work.join("cache");
        fs::create_dir(&cache).unwrap();
        let new = resource("StarRail.exe", &[b"new program"]);
        fs::write(work.root.join("StarRail.exe"), b"old").unwrap();
        fs::write(
            transfer::cache_path(&cache, &new.asset.chunks[0]).unwrap(),
            b"new program",
        )
        .unwrap();
        let task = Task::new("fixture".into(), String::new());
        let http = client::http().unwrap();
        assemble(
            &new,
            None,
            &work,
            &cache,
            "StarRail.exe",
            &http,
            &task,
            None,
        )
        .unwrap();
        assert!(!work.stage.join("StarRail.exe").exists());
        assert_eq!(fs::read(work.root.join("StarRail.exe")).unwrap(), b"old");
        assert!(transfer::cache_path(&cache, &new.asset.chunks[0])
            .unwrap()
            .is_file());
        assemble(
            &new,
            None,
            &work,
            &cache,
            "StarRail.exe",
            &http,
            &task,
            Some(&transfer::StreamingCache::new(&cache, new.asset.chunks.iter()).unwrap()),
        )
        .unwrap();
        assert_eq!(
            fs::read(work.stage.join("StarRail.exe")).unwrap(),
            b"new program"
        );
        assert_eq!(fs::read(work.root.join("StarRail.exe")).unwrap(), b"old");
        assert!(!transfer::cache_path(&cache, &new.asset.chunks[0])
            .unwrap()
            .exists());
    }

    #[test]
    fn streaming_resume_validates_partial_stage_and_frees_only_consumed_chunks() {
        let temp = Temp::new();
        let work = Workspace::open(&temp.0.join("game")).unwrap();
        let cache = work.work.join("cache");
        fs::create_dir(&cache).unwrap();
        let new = resource("game.exe", &[b"AB", b"CD", b"EF"]);
        let task = Task::new("fixture".into(), String::new());
        let http = client::http().unwrap();
        // 第一个块已落盘，第二个块暂存损坏但有预下载缓存，第三块暂时无法下载。
        fs::write(work.stage.join("game.exe"), b"ABxx").unwrap();
        let second = transfer::cache_path(&cache, &new.asset.chunks[1]).unwrap();
        fs::write(&second, b"CD").unwrap();
        let usage = transfer::StreamingCache::new(&cache, new.asset.chunks.iter()).unwrap();
        assert!(assemble(
            &new,
            None,
            &work,
            &cache,
            "game.exe",
            &http,
            &task,
            Some(&usage)
        )
        .is_err());
        assert_eq!(fs::read(work.stage.join("game.exe")).unwrap(), b"ABCD");
        assert!(!second.exists());
        assert!(!work.root.join("game.exe").exists());
        // 重启任务后前两块只能从暂存复用，不能要求下载过的压缩块仍存在。
        fs::write(
            transfer::cache_path(&cache, &new.asset.chunks[2]).unwrap(),
            b"EF",
        )
        .unwrap();
        let usage = transfer::StreamingCache::new(&cache, new.asset.chunks.iter()).unwrap();
        assemble(
            &new,
            None,
            &work,
            &cache,
            "game.exe",
            &http,
            &task,
            Some(&usage),
        )
        .unwrap();
        assert_eq!(fs::read(work.stage.join("game.exe")).unwrap(), b"ABCDEF");
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
        assert!(!work.root.join("game.exe").exists());
    }

    #[test]
    fn shared_chunk_survives_until_all_concurrent_consumers_have_staged_it() {
        let temp = Temp::new();
        let work = Workspace::open(&temp.0.join("game")).unwrap();
        let cache = work.work.join("cache");
        fs::create_dir(&cache).unwrap();
        let first = resource("first", &[b"shared"]);
        let second = resource("second", &[b"shared"]);
        let path = transfer::cache_path(&cache, &first.asset.chunks[0]).unwrap();
        fs::write(&path, b"shared").unwrap();
        let task = Task::new("fixture".into(), String::new());
        let http = client::http().unwrap();
        let usage = transfer::StreamingCache::new(
            &cache,
            first.asset.chunks.iter().chain(&second.asset.chunks),
        )
        .unwrap();
        std::thread::scope(|scope| {
            for r in [&first, &second] {
                let usage = &usage;
                let http = &http;
                let task = &task;
                let work = &work;
                let cache = &cache;
                scope.spawn(move || {
                    assemble(r, None, work, cache, &r.asset.name, http, task, Some(usage)).unwrap()
                });
            }
        });
        assert!(!path.exists());
        assert_eq!(fs::read(work.stage.join("first")).unwrap(), b"shared");
        assert_eq!(fs::read(work.stage.join("second")).unwrap(), b"shared");
    }

    #[test]
    fn deleted_categories_and_blacklist_preserve_game_resource_choices() {
        let temp = Temp::new();
        fs::write(temp.0.join("categories"), "{\"category\":\"10302\",\"is_delete\":true}\n{\"category\":\"game\",\"is_delete\":true}\n").unwrap();
        fs::write(
            temp.0.join("blacklist"),
            "{\"fileName\":\"Game_Data/Old/scene.bin\"}\n",
        )
        .unwrap();
        let excluded = exclusions(&temp.0, &json!({"res_category_dir":"categories","blacklist_dir":"blacklist","enable_resource_blacklist":true})).unwrap();
        assert!(excluded.categories.contains("10302"));
        assert!(!excluded.categories.contains("game"));
        assert!(excluded.files.contains("game_data/old/scene.bin"));
        fs::write(temp.0.join("blacklist"), "{\"fileName\":\"../outside\"}\n").unwrap();
        assert!(exclusions(
            &temp.0,
            &json!({"blacklist_dir":"blacklist","enable_resource_blacklist":true})
        )
        .is_err());
    }
}
