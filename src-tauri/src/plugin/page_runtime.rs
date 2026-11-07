use super::registry::{ExternalDependencyStatus, InstalledPlugin, PluginRegistry};
use super::{
    DeclarativePageField, LocalPageServiceContribution, PluginPageContribution, PluginPermission,
};
use crate::config::path_manager::PathManager;
use serde::Serialize;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const HEALTH_TIMEOUT: Duration = Duration::from_millis(700);
const LOCAL_SERVICE_FEATURE: &str = "local-service";
const SERVICE_IDLE_GRACE: Duration = Duration::from_secs(30);
const LEASE_TTL: Duration = Duration::from_secs(120);
const LEASE_SWEEP_INTERVAL: Duration = Duration::from_secs(10);
const STATIC_HTTP_HEADER_LIMIT: usize = 16 * 1024;
const STATIC_FILE_LIMIT: u64 = 64 * 1024 * 1024;

pub const PLUGIN_PAGE_BRIDGE_PROTOCOL_VERSION: u32 = 1;
pub const PLUGIN_PAGE_BRIDGE_REQUEST_TYPE: &str = "ssmt:capability-request";
pub const PLUGIN_PAGE_BRIDGE_RESPONSE_TYPE: &str = "ssmt:capability-response";

static LEASE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub struct PluginPageRuntime {
    shared: Arc<PluginPageRuntimeShared>,
}

#[derive(Default)]
pub struct PluginPageRuntimeShared {
    services: Mutex<HashMap<String, Arc<Mutex<ServiceSlot>>>>,
    leases: Mutex<HashMap<String, Arc<Mutex<ServiceSlot>>>>,
    static_leases: Mutex<HashMap<String, PageLeaseIdentity>>,
    static_servers: Mutex<HashMap<String, StaticPageServer>>,
}

impl Default for PluginPageRuntime {
    fn default() -> Self {
        let shared = Arc::new(PluginPageRuntimeShared::default());
        let weak = Arc::downgrade(&shared);
        thread::spawn(move || {
            while let Some(shared) = weak.upgrade() {
                thread::sleep(LEASE_SWEEP_INTERVAL);
                shared.sweep_expired();
            }
        });
        Self { shared }
    }
}

impl Deref for PluginPageRuntime {
    type Target = PluginPageRuntimeShared;

    fn deref(&self) -> &Self::Target {
        &self.shared
    }
}

impl Drop for PluginPageRuntimeShared {
    fn drop(&mut self) {
        if let Ok(mut services) = self.services.lock() {
            for service in services.values() {
                if let Ok(mut service) = service.lock() {
                    service.stop();
                }
            }
            services.clear();
        }
        if let Ok(mut servers) = self.static_servers.lock() {
            for server in servers.values_mut() {
                server.stop();
            }
            servers.clear();
        }
        if let Ok(mut leases) = self.leases.lock() {
            leases.clear();
        }
        if let Ok(mut leases) = self.static_leases.lock() {
            leases.clear();
        }
    }
}

impl PluginPageRuntime {
    pub fn lease_identity(&self, lease_id: &str) -> Result<Option<PageLeaseIdentity>, String> {
        let Some(service_slot) = self
            .leases
            .lock()
            .map_err(|_| "插件服务租约表锁已损坏".to_string())?
            .get(lease_id)
            .cloned()
        else {
            return self
                .static_leases
                .lock()
                .map_err(|_| "插件静态页面租约表锁已损坏".to_string())
                .map(|leases| leases.get(lease_id).cloned());
        };
        let slot = service_slot
            .lock()
            .map_err(|_| "插件服务实例锁已损坏".to_string())?;
        let Some(last_seen) = slot.leases.get(lease_id).copied() else {
            return Ok(None);
        };
        if last_seen.elapsed() > LEASE_TTL {
            return Ok(None);
        }
        let Some(base) = slot.base.as_ref() else {
            return Ok(None);
        };
        Ok(Some(PageLeaseIdentity {
            plugin_id: base.plugin_id.clone(),
            plugin_version: base.plugin_version.clone(),
            game_name: base.game_name.clone(),
            page_id: base.page_id.clone(),
            service_key: base.service_key.clone(),
            last_seen,
        }))
    }

    pub fn touch_lease(&self, lease_id: &str) -> Result<bool, String> {
        let service_slot = self
            .leases
            .lock()
            .map_err(|_| "插件服务租约表锁已损坏".to_string())?
            .get(lease_id)
            .cloned();
        if let Some(service_slot) = service_slot {
            let mut slot = service_slot
                .lock()
                .map_err(|_| "插件服务实例锁已损坏".to_string())?;
            let Some(last_seen) = slot.leases.get(lease_id).copied() else {
                return Ok(false);
            };
            if last_seen.elapsed() > LEASE_TTL {
                return Ok(false);
            }
            slot.leases.insert(lease_id.to_string(), Instant::now());
            return Ok(true);
        }
        let mut leases = self
            .static_leases
            .lock()
            .map_err(|_| "插件静态页面租约表锁已损坏".to_string())?;
        let Some(identity) = leases.get_mut(lease_id) else {
            return Ok(false);
        };
        if identity.last_seen.elapsed() > LEASE_TTL {
            leases.remove(lease_id);
            drop(leases);
            if let Ok(mut servers) = self.static_servers.lock() {
                if let Some(mut server) = servers.remove(lease_id) {
                    server.stop();
                }
            }
            return Ok(false);
        }
        identity.last_seen = Instant::now();
        Ok(true)
    }

    pub fn stop_plugin(&self, plugin_id: &str) -> Result<usize, String> {
        let service_slots: Vec<Arc<Mutex<ServiceSlot>>> = self
            .services
            .lock()
            .map_err(|_| "插件服务注册表锁已损坏".to_string())?
            .values()
            .cloned()
            .collect();
        let mut lease_ids = Vec::new();
        let mut stopped = 0;
        for service in service_slots {
            let mut service = service
                .lock()
                .map_err(|_| "插件服务实例锁已损坏".to_string())?;
            if service
                .base
                .as_ref()
                .is_some_and(|base| base.plugin_id == plugin_id)
            {
                lease_ids.extend(service.leases.drain().map(|(lease_id, _)| lease_id));
                service.stop();
                stopped += 1;
            }
        }
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "插件服务租约表锁已损坏".to_string())?;
        for lease_id in lease_ids {
            leases.remove(&lease_id);
        }
        let static_lease_ids: Vec<String> = {
            let mut leases = self
                .static_leases
                .lock()
                .map_err(|_| "插件静态页面租约表锁已损坏".to_string())?;
            let ids = leases
                .iter()
                .filter_map(|(lease_id, lease)| {
                    if lease.plugin_id == plugin_id {
                        Some(lease_id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            leases.retain(|_, lease| lease.plugin_id != plugin_id);
            ids
        };
        if let Ok(mut servers) = self.static_servers.lock() {
            for lease_id in static_lease_ids {
                if let Some(mut server) = servers.remove(&lease_id) {
                    server.stop();
                }
            }
        }
        Ok(stopped)
    }
}

#[derive(Debug, Clone)]
pub struct PageLeaseIdentity {
    pub plugin_id: String,
    pub plugin_version: String,
    pub game_name: String,
    pub page_id: String,
    pub service_key: String,
    last_seen: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageCatalogEntry {
    pub plugin_id: String,
    pub plugin_version: String,
    pub page_id: String,
    pub title: String,
    pub kind: String,
    pub description: String,
    pub page_path: Option<String>,
    pub fields: Vec<DeclarativePageField>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageServiceInfo {
    /// 页面或其他消费者持有的租约。旧前端兼容字段 sessionId 与 leaseId 相同。
    pub session_id: String,
    pub lease_id: String,
    pub service_key: String,
    pub lease_count: usize,
    pub plugin_id: String,
    pub plugin_version: String,
    pub page_id: String,
    pub game_name: String,
    pub running: bool,
    pub process_id: u32,
    pub url: String,
    pub page_path: String,
    pub data_root: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageServiceStatus {
    pub service_key: String,
    pub lease_count: usize,
    pub plugin_id: String,
    pub plugin_version: String,
    pub page_id: String,
    pub game_name: String,
    pub running: bool,
    pub process_id: u32,
    pub url: String,
    pub page_path: String,
    pub data_root: String,
    pub log_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticPluginPageLease {
    pub lease_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub game_name: String,
    pub page_id: String,
    pub page_path: String,
    pub url: String,
}

#[derive(Clone)]
struct ServiceBase {
    service_key: String,
    plugin_id: String,
    plugin_version: String,
    page_id: String,
    game_name: String,
    process_id: u32,
    url: String,
    page_path: String,
    data_root: String,
    log_path: String,
}

struct ServiceSlot {
    base: Option<ServiceBase>,
    process: Option<ProcessTree>,
    leases: HashMap<String, Instant>,
    health_path: String,
    token_header: Option<String>,
    idle_generation: u64,
}

impl ServiceSlot {
    fn new() -> Self {
        Self {
            base: None,
            process: None,
            leases: HashMap::new(),
            health_path: String::new(),
            token_header: None,
            idle_generation: 0,
        }
    }

    fn healthy(&mut self) -> bool {
        let Some(process) = self.process.as_mut() else {
            return false;
        };
        let Some(base) = self.base.as_ref() else {
            return false;
        };
        process_is_running(process)
            && health_ok(
                &base.url,
                self.token_header.as_deref(),
                token_from_url(&base.url),
                self.health_path.as_str(),
            )
    }

    fn stop(&mut self) {
        self.idle_generation = self.idle_generation.wrapping_add(1);
        if let Some(mut process) = self.process.take() {
            process.stop();
        }
        self.base = None;
        self.leases.clear();
    }

    fn info_for(&self, lease_id: &str) -> Option<PluginPageServiceInfo> {
        let base = self.base.as_ref()?;
        Some(PluginPageServiceInfo {
            session_id: lease_id.to_string(),
            lease_id: lease_id.to_string(),
            service_key: base.service_key.clone(),
            lease_count: self.leases.len(),
            plugin_id: base.plugin_id.clone(),
            plugin_version: base.plugin_version.clone(),
            page_id: base.page_id.clone(),
            game_name: base.game_name.clone(),
            running: true,
            process_id: base.process_id,
            url: format!("{}&leaseId={lease_id}", base.url),
            page_path: base.page_path.clone(),
            data_root: base.data_root.clone(),
            log_path: base.log_path.clone(),
        })
    }

    fn status(&self) -> Option<PluginPageServiceStatus> {
        let base = self.base.as_ref()?;
        Some(PluginPageServiceStatus {
            service_key: base.service_key.clone(),
            lease_count: self.leases.len(),
            plugin_id: base.plugin_id.clone(),
            plugin_version: base.plugin_version.clone(),
            page_id: base.page_id.clone(),
            game_name: base.game_name.clone(),
            running: true,
            process_id: base.process_id,
            url: base.url.clone(),
            page_path: base.page_path.clone(),
            data_root: base.data_root.clone(),
            log_path: base.log_path.clone(),
        })
    }
}

struct StaticPageServer {
    stop: Arc<AtomicBool>,
}

impl StaticPageServer {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for StaticPageServer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl PluginPageRuntimeShared {
    fn sweep_expired(&self) {
        let services = self
            .services
            .lock()
            .map(|services| services.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut expired_leases = Vec::new();
        let mut idle_cleanup = Vec::new();
        for service in services {
            let Ok(mut slot) = service.lock() else {
                continue;
            };
            let expired: Vec<String> = slot
                .leases
                .iter()
                .filter_map(|(lease_id, last_seen)| {
                    if last_seen.elapsed() > LEASE_TTL {
                        Some(lease_id.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let had_expired = !expired.is_empty();
            for lease_id in expired {
                slot.leases.remove(&lease_id);
                expired_leases.push(lease_id);
            }
            if had_expired && slot.leases.is_empty() && slot.process.is_some() {
                slot.idle_generation = slot.idle_generation.wrapping_add(1);
                let generation = slot.idle_generation;
                drop(slot);
                idle_cleanup.push((service, generation));
            }
        }
        for (service, generation) in idle_cleanup {
            thread::spawn(move || {
                thread::sleep(SERVICE_IDLE_GRACE);
                if let Ok(mut slot) = service.lock() {
                    if slot.leases.is_empty() && slot.idle_generation == generation {
                        slot.stop();
                    }
                }
            });
        }
        if let Ok(mut leases) = self.leases.lock() {
            for lease_id in expired_leases {
                leases.remove(&lease_id);
            }
        }

        let expired_static: Vec<String> = self
            .static_leases
            .lock()
            .map(|leases| {
                leases
                    .iter()
                    .filter_map(|(lease_id, identity)| {
                        if identity.last_seen.elapsed() > LEASE_TTL {
                            Some(lease_id.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if expired_static.is_empty() {
            return;
        }
        if let Ok(mut leases) = self.static_leases.lock() {
            for lease_id in &expired_static {
                leases.remove(lease_id);
            }
        }
        if let Ok(mut servers) = self.static_servers.lock() {
            for lease_id in expired_static {
                if let Some(mut server) = servers.remove(&lease_id) {
                    server.stop();
                }
            }
        }
    }
}

struct GamePageContext {
    game_name: String,
    game_directory: PathBuf,
    game_executable: Option<PathBuf>,
    migoto_root: PathBuf,
}

#[tauri::command]
pub fn plugin_page_catalog(game_name: String) -> Result<Vec<PluginPageCatalogEntry>, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let scoped = registry.for_game(&game_name);
    let mut entries = Vec::new();
    for plugin in scoped.selected_plugins().filter(|plugin| plugin.enabled) {
        for page in &plugin.manifest.contributions.pages {
            let absolute_path = page.path().map(|path| {
                plugin
                    .package_root
                    .join(path)
                    .to_string_lossy()
                    .into_owned()
            });
            let (description, fields) = match page {
                PluginPageContribution::Declarative {
                    description,
                    fields,
                    ..
                } => (description.clone(), fields.clone()),
                _ => (String::new(), Vec::new()),
            };
            entries.push(PluginPageCatalogEntry {
                plugin_id: plugin.id().to_string(),
                plugin_version: plugin.version().to_string(),
                page_id: page.id().to_string(),
                title: page.title().to_string(),
                kind: page.kind().to_string(),
                description,
                page_path: absolute_path,
                fields,
            });
        }
    }
    Ok(entries)
}

#[tauri::command]
pub fn acquire_static_plugin_page_lease(
    plugin_id: String,
    page_id: String,
    game_name: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<StaticPluginPageLease, String> {
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let scoped = registry.for_game(&game_name);
    let plugin = scoped
        .find(&plugin_id)
        .filter(|plugin| plugin.enabled)
        .ok_or_else(|| format!("插件未安装或未启用：{plugin_id}"))?;
    let page = plugin
        .manifest
        .contributions
        .pages
        .iter()
        .find(|page| page.id() == page_id)
        .ok_or_else(|| format!("插件页面不存在：{plugin_id}/{page_id}"))?;
    let PluginPageContribution::Static { path, .. } = page else {
        return Err("requested plugin page is not a static page".to_string());
    };
    super::validate_package_relative_path(path).map_err(|error| error.to_string())?;
    let page_path = plugin.package_root.join(path);
    if !page_path.is_file() {
        return Err(format!(
            "plugin static page is missing: {}",
            page_path.display()
        ));
    }
    let page_root = page_path
        .parent()
        .ok_or_else(|| "plugin static page has no package directory".to_string())?
        .to_path_buf();
    let page_file_name = page_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "plugin static page has an invalid filename".to_string())?;
    let page_file_name = encode_url_path(page_file_name);
    let token = create_instance_token();
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("无法分配插件静态页面端口：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("无法读取插件静态页面端口：{error}"))?
        .port();
    let network_allowed = plugin
        .manifest
        .permissions
        .iter()
        .any(|permission| *permission == PluginPermission::Network);
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server_token = token.clone();
    thread::spawn(move || {
        serve_static_page(
            listener,
            page_root,
            server_token,
            network_allowed,
            server_stop,
        )
    });
    let lease_id = next_lease_id();
    let url = format!("http://127.0.0.1:{port}/{token}/{page_file_name}");
    let identity = PageLeaseIdentity {
        plugin_id: plugin_id.clone(),
        plugin_version: plugin.manifest.version.clone(),
        game_name: game_name.clone(),
        page_id: page_id.clone(),
        service_key: String::new(),
        last_seen: Instant::now(),
    };
    let mut servers = state
        .static_servers
        .lock()
        .map_err(|_| "插件静态页面服务表锁已损坏".to_string())?;
    servers.insert(lease_id.clone(), StaticPageServer { stop });
    drop(servers);
    state
        .static_leases
        .lock()
        .map_err(|_| "插件静态页面租约表锁已损坏".to_string())?
        .insert(lease_id.clone(), identity);
    Ok(StaticPluginPageLease {
        lease_id,
        plugin_id,
        plugin_version: plugin.manifest.version.clone(),
        game_name,
        page_id,
        page_path: page_path.to_string_lossy().into_owned(),
        url,
    })
}

#[tauri::command]
pub fn release_static_plugin_page_lease(
    lease_id: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<bool, String> {
    let removed = state
        .static_leases
        .lock()
        .map_err(|_| "插件静态页面租约表锁已损坏".to_string())?
        .remove(&lease_id)
        .is_some();
    if let Ok(mut servers) = state.static_servers.lock() {
        if let Some(mut server) = servers.remove(&lease_id) {
            server.stop();
        }
    }
    Ok(removed)
}

#[tauri::command]
pub fn start_plugin_page_service(
    plugin_id: String,
    page_id: String,
    game_name: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<PluginPageServiceInfo, String> {
    acquire_plugin_page_lease_inner(&state, &plugin_id, &page_id, &game_name)
}

#[tauri::command]
pub fn acquire_plugin_page_lease(
    plugin_id: String,
    page_id: String,
    game_name: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<PluginPageServiceInfo, String> {
    acquire_plugin_page_lease_inner(&state, &plugin_id, &page_id, &game_name)
}

fn acquire_plugin_page_lease_inner(
    state: &PluginPageRuntime,
    plugin_id: &str,
    page_id: &str,
    game_name: &str,
) -> Result<PluginPageServiceInfo, String> {
    let game = load_game_page_context(game_name)?;
    let registry = PluginRegistry::from_default_location().map_err(|error| error.to_string())?;
    let scoped = registry.for_game(&game.game_name);
    let plugin = scoped
        .find(plugin_id)
        .filter(|plugin| plugin.enabled)
        .ok_or_else(|| format!("插件未安装或未启用：{plugin_id}"))?;
    ensure_page_preflight(plugin, page_id)?;
    let (service_contribution, page_path) = local_service_page(plugin, page_id)?;
    let service = service_contribution.clone();
    let page_path = page_path.to_string();
    let service_key = service_key(plugin, page_id, &game.game_name);

    let service_slot = state
        .services
        .lock()
        .map_err(|_| "插件服务注册表锁已损坏".to_string())?
        .entry(service_key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(ServiceSlot::new())))
        .clone();

    let lease_id = next_lease_id();
    let mut slot = service_slot
        .lock()
        .map_err(|_| "插件服务实例锁已损坏".to_string())?;
    let mut stale_leases = Vec::new();
    if !slot.healthy() {
        stale_leases.extend(slot.leases.drain().map(|(lease_id, _)| lease_id));
        slot.stop();
        start_service_process(
            &mut slot,
            plugin,
            &game,
            &service,
            page_id,
            &page_path,
            &service_key,
        )?;
    }
    slot.idle_generation = slot.idle_generation.wrapping_add(1);
    slot.leases.insert(lease_id.clone(), Instant::now());
    let info = slot
        .info_for(&lease_id)
        .ok_or_else(|| "插件服务启动后缺少运行信息".to_string())?;
    drop(slot);
    let mut leases = state
        .leases
        .lock()
        .map_err(|_| "插件服务租约表锁已损坏".to_string())?;
    for stale in stale_leases {
        leases.remove(&stale);
    }
    leases.insert(lease_id, service_slot);
    drop(leases);
    Ok(info)
}

#[tauri::command]
pub fn release_plugin_page_lease(
    lease_id: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<bool, String> {
    let Some(service_slot) = state
        .leases
        .lock()
        .map_err(|_| "插件服务租约表锁已损坏".to_string())?
        .remove(&lease_id)
    else {
        return Ok(false);
    };
    let cleanup = {
        let mut slot = service_slot
            .lock()
            .map_err(|_| "插件服务实例锁已损坏".to_string())?;
        if slot.leases.remove(&lease_id).is_none() {
            return Ok(false);
        }
        if slot.leases.is_empty() && slot.process.is_some() {
            slot.idle_generation = slot.idle_generation.wrapping_add(1);
            Some(slot.idle_generation)
        } else {
            slot.idle_generation = slot.idle_generation.wrapping_add(1);
            None
        }
    };
    if let Some(generation) = cleanup {
        let service_slot = service_slot.clone();
        thread::spawn(move || {
            thread::sleep(SERVICE_IDLE_GRACE);
            if let Ok(mut slot) = service_slot.lock() {
                if slot.leases.is_empty() && slot.idle_generation == generation {
                    slot.stop();
                }
            }
        });
    }
    Ok(true)
}

#[tauri::command]
pub fn touch_plugin_page_lease(
    lease_id: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<bool, String> {
    state.touch_lease(&lease_id)
}

#[tauri::command]
pub fn stop_plugin_page_service(
    session_id: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<bool, String> {
    let Some(service_slot) = state
        .leases
        .lock()
        .map_err(|_| "插件服务租约表锁已损坏".to_string())?
        .remove(&session_id)
    else {
        return Ok(false);
    };
    let mut slot = service_slot
        .lock()
        .map_err(|_| "插件服务实例锁已损坏".to_string())?;
    let lease_ids: Vec<String> = slot.leases.drain().map(|(lease_id, _)| lease_id).collect();
    slot.stop();
    drop(slot);
    let mut leases = state
        .leases
        .lock()
        .map_err(|_| "插件服务租约表锁已损坏".to_string())?;
    for lease_id in lease_ids {
        leases.remove(&lease_id);
    }
    Ok(true)
}

#[tauri::command]
pub fn stop_plugin_page_services_for_plugin(
    plugin_id: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<usize, String> {
    state.stop_plugin(&plugin_id)
}

#[tauri::command]
pub fn stop_all_plugin_page_services(state: State<'_, PluginPageRuntime>) -> Result<usize, String> {
    let services: Vec<_> = state
        .services
        .lock()
        .map_err(|_| "插件服务注册表锁已损坏".to_string())?
        .values()
        .cloned()
        .collect();
    let stopped = services.len();
    for service in services {
        service
            .lock()
            .map_err(|_| "插件服务实例锁已损坏".to_string())?
            .stop();
    }
    state
        .services
        .lock()
        .map_err(|_| "插件服务注册表锁已损坏".to_string())?
        .clear();
    state
        .leases
        .lock()
        .map_err(|_| "插件服务租约表锁已损坏".to_string())?
        .clear();
    if let Ok(mut servers) = state.static_servers.lock() {
        for server in servers.values_mut() {
            server.stop();
        }
        servers.clear();
    }
    state
        .static_leases
        .lock()
        .map_err(|_| "插件静态页面租约表锁已损坏".to_string())?
        .clear();
    Ok(stopped)
}

#[tauri::command]
pub fn plugin_page_session_status(
    session_id: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<Option<PluginPageServiceInfo>, String> {
    let Some(service_slot) = state
        .leases
        .lock()
        .map_err(|_| "插件服务租约表锁已损坏".to_string())?
        .get(&session_id)
        .cloned()
    else {
        return Ok(None);
    };
    let mut slot = service_slot
        .lock()
        .map_err(|_| "插件服务实例锁已损坏".to_string())?;
    let invalid = !slot.leases.contains_key(&session_id) || !slot.healthy();
    if invalid {
        drop(slot);
        state
            .leases
            .lock()
            .map_err(|_| "插件服务租约表锁已损坏".to_string())?
            .remove(&session_id);
        return Ok(None);
    }
    Ok(slot.info_for(&session_id))
}

#[tauri::command]
pub fn plugin_page_service_status(
    service_key: String,
    state: State<'_, PluginPageRuntime>,
) -> Result<Option<PluginPageServiceStatus>, String> {
    let Some(service_slot) = state
        .services
        .lock()
        .map_err(|_| "插件服务注册表锁已损坏".to_string())?
        .get(&service_key)
        .cloned()
    else {
        return Ok(None);
    };
    let mut slot = service_slot
        .lock()
        .map_err(|_| "插件服务实例锁已损坏".to_string())?;
    if !slot.healthy() {
        let stale_leases: Vec<String> = slot.leases.drain().map(|(lease_id, _)| lease_id).collect();
        slot.stop();
        drop(slot);
        let mut leases = state
            .leases
            .lock()
            .map_err(|_| "插件服务租约表锁已损坏".to_string())?;
        for lease_id in stale_leases {
            leases.remove(&lease_id);
        }
        return Ok(None);
    }
    Ok(slot.status())
}

fn start_service_process(
    slot: &mut ServiceSlot,
    plugin: &InstalledPlugin,
    game: &GamePageContext,
    service: &LocalPageServiceContribution,
    page_id: &str,
    page_path: &str,
    service_key: &str,
) -> Result<(), String> {
    let executable = resolve_page_template(&service.executable, plugin, game, None, None, None)?;
    let executable = PathBuf::from(executable);
    if !executable.is_file() {
        return Err(format!("插件页面服务程序不存在：{}", executable.display()));
    }
    let working_directory = match service.working_directory.as_deref() {
        Some(value) => PathBuf::from(resolve_page_template(
            value, plugin, game, None, None, None,
        )?),
        None => plugin.package_root.clone(),
    };
    if !working_directory.is_dir() {
        return Err(format!(
            "插件页面服务工作目录不存在：{}",
            working_directory.display()
        ));
    }

    let mods_root = game.migoto_root.join("Mods");
    if !mods_root.is_dir() {
        return Err(format!("找不到游戏 Mods 目录：{}", mods_root.display()));
    }
    let data_root = mods_root.join(&service.data_directory);
    fs::create_dir_all(&data_root).map_err(|error| format!("无法创建插件页面数据目录：{error}"))?;
    let logs_root = data_root.join(".logs");
    fs::create_dir_all(&logs_root).map_err(|error| format!("无法创建插件页面日志目录：{error}"))?;

    let port = reserve_loopback_port()?;
    let token = create_instance_token();
    let log_path = logs_root.join(format!("{}.log", safe_log_name(service_key)));
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("无法打开插件页面服务日志：{error}"))?;
    let stdout = log
        .try_clone()
        .map_err(|error| format!("无法复制插件页面服务日志句柄：{error}"))?;
    let stderr = log
        .try_clone()
        .map_err(|error| format!("无法复制插件页面服务日志句柄：{error}"))?;

    let mut command = Command::new(&executable);
    for argument in &service.arguments {
        command.arg(resolve_page_template(
            argument,
            plugin,
            game,
            Some(port),
            Some(&token),
            Some(&data_root),
        )?);
    }
    command
        .current_dir(&working_directory)
        .env("SSMT_PLUGIN_ID", &plugin.manifest.id)
        .env("SSMT_PLUGIN_PAGE_ID", page_id)
        .env("SSMT_PLUGIN_VERSION", &plugin.manifest.version)
        .env("SSMT_GAME_NAME", &game.game_name)
        .env("SSMT_GAME_MODS", &mods_root)
        .env("SSMT_SERVICE_PORT", port.to_string())
        .env("SSMT_SERVICE_TOKEN", &token)
        .env("SSMT_SERVICE_DATA", &data_root)
        .env("SSMT_PARENT_PID", std::process::id().to_string())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let child = spawn_page_process(&mut command)?;
    let mut process = ProcessTree::new(child)?;
    let url = page_service_url(
        page_path,
        port,
        &token,
        &game.game_name,
        service_key,
        plugin.id(),
    );
    let deadline = Instant::now() + Duration::from_millis(service.ready_timeout_ms);
    while Instant::now() < deadline {
        if !process_is_running(&mut process) {
            return Err(format!(
                "插件页面服务提前退出。日志：{}",
                log_path.display()
            ));
        }
        if health_ok(
            &url,
            service.token_header.as_deref(),
            &token,
            &service.health_path,
        ) {
            slot.base = Some(ServiceBase {
                service_key: service_key.to_string(),
                plugin_id: plugin.id().to_string(),
                plugin_version: plugin.version().to_string(),
                page_id: page_id.to_string(),
                game_name: game.game_name.clone(),
                process_id: process.child.id(),
                url,
                page_path: page_path.to_string(),
                data_root: data_root.to_string_lossy().into_owned(),
                log_path: log_path.to_string_lossy().into_owned(),
            });
            slot.process = Some(process);
            slot.health_path = service.health_path.clone();
            slot.token_header = service.token_header.clone();
            return Ok(());
        }
        thread::sleep(Duration::from_millis(180));
    }
    process.stop();
    Err(format!(
        "等待插件页面服务启动超时。日志：{}",
        log_path.display()
    ))
}

fn ensure_page_preflight(plugin: &InstalledPlugin, page_id: &str) -> Result<(), String> {
    if !plugin.manifest.supports_current_plugin_api() {
        return Err("插件页面 API 版本不受支持".to_string());
    }
    if !plugin
        .manifest
        .permissions
        .iter()
        .any(|permission| *permission == PluginPermission::ProcessSpawn)
    {
        return Err("插件页面服务缺少 process.spawn 权限".to_string());
    }
    for dependency in &plugin.manifest.external_dependencies {
        if !plugin
            .manifest
            .requires_dependency(&dependency.id, LOCAL_SERVICE_FEATURE)
        {
            continue;
        }
        let ready = plugin
            .external_dependencies
            .get(&dependency.id)
            .is_some_and(|state| {
                state.status == ExternalDependencyStatus::Ready && state.path.is_some()
            });
        if !ready {
            return Err(format!("插件页面外部依赖未就绪：{}", dependency.id));
        }
    }
    let _ = local_service_page(plugin, page_id)?;
    Ok(())
}

fn local_service_page<'a>(
    plugin: &'a InstalledPlugin,
    page_id: &str,
) -> Result<(&'a LocalPageServiceContribution, &'a str), String> {
    for page in &plugin.manifest.contributions.pages {
        if page.id() != page_id {
            continue;
        }
        if let PluginPageContribution::LocalService { service, path, .. } = page {
            return Ok((service, path));
        }
        return Err(format!("插件页面不是本地服务类型：{page_id}"));
    }
    Err(format!("插件没有页面：{page_id}"))
}

fn load_game_page_context(game_name: &str) -> Result<GamePageContext, String> {
    let config_path = PathManager::global_config_games_game_folder(game_name).join("Config.json");
    let raw = fs::read_to_string(&config_path)
        .map_err(|error| format!("无法读取游戏配置 {}：{error}", config_path.display()))?;
    let config = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|error| format!("无法解析游戏配置：{error}"))?;
    let migoto_root = config
        .get("installDir")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "游戏配置缺少 3Dmigoto installDir".to_string())?;
    let game_executable = config
        .get("targetExePath")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let game_directory = game_executable
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| migoto_root.clone());
    Ok(GamePageContext {
        game_name: game_name.to_string(),
        game_directory,
        game_executable,
        migoto_root,
    })
}

fn service_key(plugin: &InstalledPlugin, page_id: &str, game_name: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        plugin.id(),
        plugin.version(),
        page_id,
        game_name.to_ascii_lowercase()
    )
}

fn next_lease_id() -> String {
    format!(
        "lease-{}-{}",
        std::process::id(),
        LEASE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn safe_log_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn resolve_page_template(
    template: &str,
    plugin: &InstalledPlugin,
    game: &GamePageContext,
    port: Option<u16>,
    token: Option<&str>,
    data_root: Option<&Path>,
) -> Result<String, String> {
    let mut resolved = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find("${") {
        resolved.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find('}') else {
            return Err(format!("页面模板变量未闭合：{template}"));
        };
        let variable = &after_start[..end];
        let replacement = match variable {
            "plugin.root" => plugin.package_root.to_string_lossy().into_owned(),
            "game.exe" => game
                .game_executable
                .as_deref()
                .ok_or_else(|| "游戏配置缺少 targetExePath".to_string())?
                .to_string_lossy()
                .into_owned(),
            "game.directory" => game.game_directory.to_string_lossy().into_owned(),
            "game.mods" => game.migoto_root.join("Mods").to_string_lossy().into_owned(),
            "service.port" => port
                .ok_or_else(|| "服务端口变量不可用".to_string())?
                .to_string(),
            "service.token" => token
                .ok_or_else(|| "服务令牌变量不可用".to_string())?
                .to_string(),
            "service.data" => data_root
                .ok_or_else(|| "服务数据目录变量不可用".to_string())?
                .to_string_lossy()
                .into_owned(),
            "host.pid" => std::process::id().to_string(),
            _ => {
                let dependency_id = variable
                    .strip_prefix("external.")
                    .ok_or_else(|| format!("不支持的页面模板变量：${{{variable}}}"))?;
                let state = plugin
                    .external_dependencies
                    .get(dependency_id)
                    .ok_or_else(|| format!("缺少外部依赖：{dependency_id}"))?;
                state
                    .path
                    .as_deref()
                    .ok_or_else(|| format!("外部依赖路径未就绪：{dependency_id}"))?
                    .to_string_lossy()
                    .into_owned()
            }
        };
        resolved.push_str(&replacement);
        remaining = &after_start[end + 1..];
    }
    if remaining.contains('}') {
        return Err(format!("页面模板包含多余字符：{template}"));
    }
    resolved.push_str(remaining);
    Ok(resolved)
}

fn serve_static_page(
    listener: TcpListener,
    root: PathBuf,
    token: String,
    network_allowed: bool,
    stop: Arc<AtomicBool>,
) {
    let _ = listener.set_nonblocking(true);
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) =
                    handle_static_page_request(stream, &root, &token, network_allowed)
                {
                    eprintln!("[PluginPage] Static page request failed: {error}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                eprintln!("[PluginPage] Static page server stopped: {error}");
                break;
            }
        }
    }
}

fn handle_static_page_request(
    mut stream: TcpStream,
    root: &Path,
    token: &str,
    network_allowed: bool,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while request.len() < STATIC_HTTP_HEADER_LIMIT
        && !request.windows(4).any(|window| window == b"\r\n\r\n")
    {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    if request.len() > STATIC_HTTP_HEADER_LIMIT {
        return write_static_response(
            &mut stream,
            431,
            "text/plain",
            b"header too large",
            false,
            network_allowed,
        );
    }
    let request_line = String::from_utf8_lossy(&request)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if !matches!(method, "GET" | "HEAD") {
        return write_static_response(
            &mut stream,
            405,
            "text/plain",
            b"method not allowed",
            false,
            network_allowed,
        );
    }
    let raw_path = target.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(raw_path)?;
    let prefix = format!("/{token}/");
    let Some(relative) = decoded.strip_prefix(&prefix) else {
        return write_static_response(
            &mut stream,
            403,
            "text/plain",
            b"forbidden",
            false,
            network_allowed,
        );
    };
    if relative.is_empty() || super::validate_package_relative_path(relative).is_err() {
        return write_static_response(
            &mut stream,
            403,
            "text/plain",
            b"forbidden",
            false,
            network_allowed,
        );
    }
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let candidate = root.join(relative);
    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        _ => {
            return write_static_response(
                &mut stream,
                404,
                "text/plain",
                b"not found",
                false,
                network_allowed,
            );
        }
    };
    if metadata.len() > STATIC_FILE_LIMIT {
        return write_static_response(
            &mut stream,
            413,
            "text/plain",
            b"file too large",
            false,
            network_allowed,
        );
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !canonical.starts_with(&root) {
        return write_static_response(
            &mut stream,
            403,
            "text/plain",
            b"forbidden",
            false,
            network_allowed,
        );
    }
    let bytes = fs::read(&canonical).map_err(|error| error.to_string())?;
    write_static_response(
        &mut stream,
        200,
        static_mime_type(&canonical),
        &bytes,
        method == "HEAD",
        network_allowed,
    )
}

fn write_static_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    network_allowed: bool,
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    let network_sources = if network_allowed { " https:" } else { "" };
    let csp = format!(
        "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:{network_sources}; media-src 'self' data: blob:{network_sources}; \
font-src 'self' data:; connect-src 'self'{network_sources}; frame-src 'none'; child-src 'none'; \
object-src 'none'; base-uri 'none'; form-action 'none'; worker-src 'self' blob:"
    );
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
Content-Security-Policy: {csp}\r\nCross-Origin-Resource-Policy: same-origin\r\n\
Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|_| {
            if head_only {
                Ok(())
            } else {
                stream.write_all(body)
            }
        })
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("incomplete percent escape".to_string());
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|error| error.to_string())
}

fn encode_url_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn hex_value(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid percent escape".to_string()),
    }
}

fn static_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn reserve_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("无法分配本机端口：{error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("无法读取本机端口：{error}"))
}

fn create_instance_token() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let random = RandomState::new();
    let mut first = random.build_hasher();
    timestamp.hash(&mut first);
    std::process::id().hash(&mut first);
    let mut second = random.build_hasher();
    LEASE_SEQUENCE.load(Ordering::Relaxed).hash(&mut second);
    format!("{:016x}{:016x}", first.finish(), second.finish())
}

fn page_service_url(
    page_path: &str,
    port: u16,
    token: &str,
    game_name: &str,
    service_key: &str,
    plugin_id: &str,
) -> String {
    let page = page_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    format!(
        "http://127.0.0.1:{port}/{page}#token={token}&game={game_name}&serviceKey={service_key}&pluginId={plugin_id}"
    )
}

fn token_from_url(url: &str) -> &str {
    url.split_once("#token=")
        .and_then(|(_, tail)| tail.split('&').next())
        .unwrap_or_default()
}

fn health_ok(url: &str, token_header: Option<&str>, token: &str, health_path: &str) -> bool {
    let Ok(base) = reqwest::Url::parse(url) else {
        return false;
    };
    let Ok(health) = base.join(health_path) else {
        return false;
    };
    let client = reqwest::blocking::Client::new();
    let mut request = client.get(health).timeout(HEALTH_TIMEOUT);
    if let Some(header) = token_header {
        request = request.header(header, token);
    }
    request
        .send()
        .is_ok_and(|response| response.status().is_success())
}

fn spawn_page_process(command: &mut Command) -> Result<Child, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        match command.spawn() {
            Err(error) if error.raw_os_error() == Some(5) => {
                command.creation_flags(CREATE_NO_WINDOW);
            }
            result => {
                return result.map_err(|error| format!("无法启动插件页面服务：{error}"));
            }
        }
    }
    command
        .spawn()
        .map_err(|error| format!("无法启动插件页面服务：{error}"))
}

fn process_is_running(process: &mut ProcessTree) -> bool {
    process.child.try_wait().ok().flatten().is_none()
}

struct ProcessTree {
    child: Child,
    job: Option<ProcessJob>,
}

impl ProcessTree {
    fn new(child: Child) -> Result<Self, String> {
        let job = match ProcessJob::attach(child.id()) {
            Ok(job) => job,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            job: Some(job),
        })
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.job.take();
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
struct ProcessJob(usize);

#[cfg(windows)]
impl ProcessJob {
    fn attach(process_id: u32) -> Result<Self, String> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };

        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err("无法创建插件页面 Job Object".to_string());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                CloseHandle(job);
                return Err("无法配置插件页面 Job Object".to_string());
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, process_id);
            if process.is_null() {
                CloseHandle(job);
                return Err("无法打开插件页面服务进程".to_string());
            }
            let attached = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if attached == 0 {
                CloseHandle(job);
                return Err("无法把插件页面服务加入 Job Object".to_string());
            }
            Ok(Self(job as usize))
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        unsafe {
            CloseHandle(self.0 as _);
        }
    }
}

#[cfg(not(windows))]
struct ProcessJob;

#[cfg(not(windows))]
impl ProcessJob {
    fn attach(_process_id: u32) -> Result<Self, String> {
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::ExternalDependencyState;
    use crate::plugin::{
        ExternalDependency, ExternalDependencyType, PluginCompatibility, PluginContributions,
        PluginManifest,
    };
    use std::collections::BTreeMap;

    fn fixture_plugin(root: &Path) -> InstalledPlugin {
        InstalledPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                id: "ssmt.page.fixture".to_string(),
                name: "Page Fixture".to_string(),
                version: "1.0.0".to_string(),
                author: "SSMT".to_string(),
                plugin_api_version: 1,
                compatibility: PluginCompatibility {
                    ssmt: ">=4.2.2".to_string(),
                    platforms: vec!["windows-x64".to_string()],
                    plugin_api: None,
                    games: vec!["EFMI".to_string()],
                },
                contributions: PluginContributions::default(),
                external_dependencies: vec![ExternalDependency {
                    id: "python-runtime".to_string(),
                    kind: ExternalDependencyType::Directory,
                    required_files: vec!["python.exe".to_string()],
                    required_for: Some(vec![LOCAL_SERVICE_FEATURE.to_string()]),
                    download_url: None,
                    download_hint: None,
                }],
                permissions: vec![PluginPermission::ProcessSpawn],
            },
            package_root: root.join("package"),
            enabled: true,
            official: true,
            external_dependencies: BTreeMap::from([(
                "python-runtime".to_string(),
                ExternalDependencyState {
                    status: ExternalDependencyStatus::Ready,
                    path: Some(root.join("python")),
                    reason: None,
                },
            )]),
        }
    }

    #[test]
    fn resolves_page_templates_without_shell_interpolation() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-page-template-{}-{}",
            std::process::id(),
            LEASE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let plugin = fixture_plugin(&root);
        let game = GamePageContext {
            game_name: "EFMI".to_string(),
            game_directory: root.join("game"),
            game_executable: Some(root.join("game/game.exe")),
            migoto_root: root.join("migoto"),
        };
        let value = resolve_page_template(
            "${external.python-runtime}/python.exe",
            &plugin,
            &game,
            Some(1234),
            Some("token"),
            Some(&root.join("data")),
        )
        .unwrap();
        assert_eq!(PathBuf::from(value), root.join("python").join("python.exe"));
    }

    #[test]
    fn static_page_server_requires_token_and_rejects_path_escape() {
        let root = std::env::temp_dir().join(format!(
            "ssmt-static-page-server-{}-{}",
            std::process::id(),
            LEASE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let page_root = root.join("ui");
        fs::create_dir_all(&page_root).unwrap();
        fs::write(page_root.join("index.html"), b"static-page-ok").unwrap();
        fs::write(root.join("secret.txt"), b"secret").unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = "fixture-token";
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = thread::spawn(move || {
            serve_static_page(listener, page_root, token.to_string(), false, thread_stop);
        });

        let request = |path: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        assert!(request("/fixture-token/index.html").starts_with("HTTP/1.1 200"));
        assert!(request("/wrong-token/index.html").starts_with("HTTP/1.1 403"));
        assert!(request("/fixture-token/../secret.txt").starts_with("HTTP/1.1 403"));

        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_static_lease_is_removed_by_the_janitor() {
        let shared = PluginPageRuntimeShared::default();
        let lease_id = "expired-static".to_string();
        shared.static_leases.lock().unwrap().insert(
            lease_id.clone(),
            PageLeaseIdentity {
                plugin_id: "ssmt.page.fixture".to_string(),
                plugin_version: "1.0.0".to_string(),
                game_name: "EFMI".to_string(),
                page_id: "settings".to_string(),
                service_key: String::new(),
                last_seen: Instant::now() - LEASE_TTL - Duration::from_secs(1),
            },
        );
        let stop = Arc::new(AtomicBool::new(false));
        shared
            .static_servers
            .lock()
            .unwrap()
            .insert(lease_id.clone(), StaticPageServer { stop: stop.clone() });

        shared.sweep_expired();

        assert!(shared.static_leases.lock().unwrap().is_empty());
        assert!(shared.static_servers.lock().unwrap().is_empty());
        assert!(stop.load(Ordering::Relaxed));
    }

    #[test]
    fn expired_service_lease_is_removed_by_the_janitor() {
        let shared = PluginPageRuntimeShared::default();
        let lease_id = "expired-service".to_string();
        let mut slot = ServiceSlot::new();
        slot.leases.insert(
            lease_id.clone(),
            Instant::now() - LEASE_TTL - Duration::from_secs(1),
        );
        let slot = Arc::new(Mutex::new(slot));
        shared
            .services
            .lock()
            .unwrap()
            .insert("fixture-service".to_string(), slot.clone());
        shared
            .leases
            .lock()
            .unwrap()
            .insert(lease_id.clone(), slot.clone());

        shared.sweep_expired();

        assert!(slot.lock().unwrap().leases.is_empty());
        assert!(shared.leases.lock().unwrap().is_empty());
    }
}
