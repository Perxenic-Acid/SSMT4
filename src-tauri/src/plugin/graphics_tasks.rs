//! 图形插件的通用后台任务代理。
//!
//! 插件页面只能提交声明式操作并轮询任务；文件校验、下载、安装和卸载仍在宿主侧执行，
//! 任务身份来自当前 PageLease，不能由 iframe 自行指定插件或游戏。

use super::dlss5_hot_runtime;
use super::gimi_hot_runtime::download;
use super::page_runtime::PageLeaseIdentity;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const TASK_TTL: Duration = Duration::from_secs(60 * 60);
const TASK_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_ACTIVE_TASKS: usize = 4;
const TASK_ID_SEQUENCE_OFFSET: u64 = 1;
static TASK_ID_SEQUENCE: AtomicU64 = AtomicU64::new(TASK_ID_SEQUENCE_OFFSET);

#[derive(Debug, Clone)]
pub(crate) struct GraphicsProgress {
    pub phase: String,
    pub file: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct GraphicsTaskOwner {
    pub plugin_id: String,
    pub plugin_version: String,
    pub game_name: String,
    pub page_id: String,
    pub service_key: String,
}

impl From<&PageLeaseIdentity> for GraphicsTaskOwner {
    fn from(identity: &PageLeaseIdentity) -> Self {
        Self {
            plugin_id: identity.plugin_id.clone(),
            plugin_version: identity.plugin_version.clone(),
            game_name: identity.game_name.clone(),
            page_id: identity.page_id.clone(),
            service_key: identity.service_key.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphicsTaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsTaskSnapshot {
    pub task_id: String,
    pub operation: String,
    pub status: GraphicsTaskStatus,
    pub phase: String,
    pub message: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub cancellable: bool,
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphicsTaskError {
    pub code: &'static str,
    pub message: String,
}

impl GraphicsTaskError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

struct GraphicsTaskRecord {
    task_id: String,
    owner: GraphicsTaskOwner,
    game_executable: String,
    operation: String,
    status: GraphicsTaskStatus,
    phase: String,
    message: String,
    downloaded_bytes: u64,
    total_bytes: u64,
    cancellable: bool,
    created_at_ms: u64,
    started_at_ms: Option<u64>,
    finished_at_ms: Option<u64>,
    result: Option<Value>,
    error_code: Option<String>,
    error_message: Option<String>,
    cancel: Option<watch::Sender<bool>>,
}

impl GraphicsTaskRecord {
    fn is_active(&self) -> bool {
        matches!(
            self.status,
            GraphicsTaskStatus::Queued | GraphicsTaskStatus::Running
        )
    }

    fn snapshot(&self) -> GraphicsTaskSnapshot {
        GraphicsTaskSnapshot {
            task_id: self.task_id.clone(),
            operation: self.operation.clone(),
            status: self.status,
            phase: self.phase.clone(),
            message: self.message.clone(),
            downloaded_bytes: self.downloaded_bytes,
            total_bytes: self.total_bytes,
            cancellable: self.cancellable,
            created_at_ms: self.created_at_ms,
            started_at_ms: self.started_at_ms,
            finished_at_ms: self.finished_at_ms,
            result: self.result.clone(),
            error_code: self.error_code.clone(),
            error_message: self.error_message.clone(),
        }
    }

    fn mark_running(&mut self) {
        self.status = GraphicsTaskStatus::Running;
        self.phase = "starting".to_string();
        self.message = "任务已开始".to_string();
        self.started_at_ms = Some(unix_millis());
    }

    fn report(&mut self, progress: GraphicsProgress) {
        if !self.is_active() {
            return;
        }
        self.status = GraphicsTaskStatus::Running;
        self.phase = progress.phase;
        self.downloaded_bytes = progress.downloaded_bytes;
        self.total_bytes = progress.total_bytes;
        self.message = if progress.file.is_empty() {
            self.phase.clone()
        } else {
            format!("{}: {}", self.phase, progress.file)
        };
    }

    fn succeed(&mut self, value: Value) {
        self.status = GraphicsTaskStatus::Succeeded;
        self.phase = "completed".to_string();
        self.message = "任务已完成".to_string();
        self.finished_at_ms = Some(unix_millis());
        self.result = Some(value);
        self.error_code = None;
        self.error_message = None;
        self.cancel = None;
    }

    fn fail(&mut self, error: String) {
        self.status = GraphicsTaskStatus::Failed;
        self.phase = "failed".to_string();
        self.message = error.clone();
        self.finished_at_ms = Some(unix_millis());
        self.error_code = Some("graphics_operation_failed".to_string());
        self.error_message = Some(error);
        self.cancel = None;
    }

    fn cancelled(&mut self) {
        self.status = GraphicsTaskStatus::Cancelled;
        self.phase = "cancelled".to_string();
        self.message = "任务已取消".to_string();
        self.finished_at_ms = Some(unix_millis());
        self.error_code = Some("task_cancelled".to_string());
        self.error_message = Some("任务已取消".to_string());
        self.cancel = None;
    }
}

#[derive(Default, Clone)]
pub struct GraphicsTaskBroker {
    tasks: Arc<Mutex<HashMap<String, Arc<Mutex<GraphicsTaskRecord>>>>>,
}

#[derive(Debug, Clone)]
enum GraphicsRuntime {
    Gimi,
    Dlss5,
}

#[derive(Debug, Clone)]
enum GraphicsTaskJob {
    RuntimeDownload {
        runtime: GraphicsRuntime,
        route: Option<String>,
        profile: String,
    },
    RuntimeImport {
        source_directory: String,
        profile: String,
    },
    RuntimeSelect {
        root: String,
    },
    RuntimeRemove {
        root: String,
    },
    RouteInstall {
        route: String,
        api_override: String,
        anti_cheat_acknowledged: bool,
    },
    RouteRestore,
}

impl GraphicsTaskJob {
    fn parse(payload: &Value) -> Result<Self, GraphicsTaskError> {
        let operation = required_string(payload, "operation")?;
        match operation {
            "runtime.download" => Ok(Self::RuntimeDownload {
                runtime: parse_runtime(payload)?,
                route: optional_string(payload, "route").map(str::to_string),
                profile: required_string(payload, "profile")?.to_string(),
            }),
            "runtime.import" => Ok(Self::RuntimeImport {
                source_directory: required_string(payload, "sourceDirectory")?.to_string(),
                profile: required_string(payload, "profile")?.to_string(),
            }),
            "runtime.select" => Ok(Self::RuntimeSelect {
                root: required_string(payload, "root")?.to_string(),
            }),
            "runtime.remove" => Ok(Self::RuntimeRemove {
                root: required_string(payload, "root")?.to_string(),
            }),
            "route.install" => Ok(Self::RouteInstall {
                route: required_string(payload, "route")?.to_string(),
                api_override: optional_string(payload, "apiOverride")
                    .unwrap_or("auto")
                    .to_string(),
                anti_cheat_acknowledged: payload
                    .get("antiCheatAcknowledged")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            "route.restore" => Ok(Self::RouteRestore),
            _ => Err(GraphicsTaskError::new(
                "invalid_payload",
                format!("不支持的图形任务操作：{operation}"),
            )),
        }
    }

    fn operation(&self) -> &'static str {
        match self {
            Self::RuntimeDownload { .. } => "runtime.download",
            Self::RuntimeImport { .. } => "runtime.import",
            Self::RuntimeSelect { .. } => "runtime.select",
            Self::RuntimeRemove { .. } => "runtime.remove",
            Self::RouteInstall { .. } => "route.install",
            Self::RouteRestore => "route.restore",
        }
    }

    fn cancellable(&self) -> bool {
        matches!(self, Self::RuntimeDownload { .. })
    }
}

impl GraphicsTaskBroker {
    pub fn start(
        &self,
        owner: GraphicsTaskOwner,
        payload: Value,
    ) -> Result<GraphicsTaskSnapshot, GraphicsTaskError> {
        let job = GraphicsTaskJob::parse(&payload)?;
        self.prune_finished();

        let task_id = format!(
            "graphics-{}-{}",
            std::process::id(),
            TASK_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let (cancel, cancel_rx) = watch::channel(false);
        let cancellable = job.cancellable();
        let game_executable = game_executable(&owner.game_name)
            .map_err(|error| GraphicsTaskError::new("graphics_task_failed", error))?;
        let owner_plugin_id = owner.plugin_id.clone();
        let owner_game_name = owner.game_name.clone();
        let record = Arc::new(Mutex::new(GraphicsTaskRecord {
            task_id: task_id.clone(),
            owner,
            game_executable,
            operation: job.operation().to_string(),
            status: GraphicsTaskStatus::Queued,
            phase: "queued".to_string(),
            message: "任务已排队".to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
            cancellable,
            created_at_ms: unix_millis(),
            started_at_ms: None,
            finished_at_ms: None,
            result: None,
            error_code: None,
            error_message: None,
            cancel: cancellable.then_some(cancel),
        }));
        let snapshot = record
            .lock()
            .map_err(|_| GraphicsTaskError::new("task_store_unavailable", "图形任务记录锁已损坏"))?
            .snapshot();
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| GraphicsTaskError::new("task_store_unavailable", "图形任务表锁已损坏"))?;
        let mut active = 0usize;
        for task in tasks.values() {
            let task = task.lock().map_err(|_| {
                GraphicsTaskError::new("task_store_unavailable", "图形任务记录锁已损坏")
            })?;
            if !task.is_active() {
                continue;
            }
            active += 1;
            if task.owner.plugin_id == owner_plugin_id && task.owner.game_name == owner_game_name {
                return Err(GraphicsTaskError::new(
                    "graphics_task_conflict",
                    format!("插件已有图形任务运行中：{}", task.task_id),
                ));
            }
        }
        if active >= MAX_ACTIVE_TASKS {
            return Err(GraphicsTaskError::new(
                "graphics_task_limit",
                "同时运行的图形任务已达上限，请等待已有任务完成",
            ));
        }
        tasks.insert(task_id, record.clone());
        drop(tasks);

        tauri::async_runtime::spawn(run_task(record, job, cancel_rx));
        Ok(snapshot)
    }

    pub fn status(
        &self,
        owner: &GraphicsTaskOwner,
        task_id: &str,
    ) -> Result<GraphicsTaskSnapshot, GraphicsTaskError> {
        let record = self.authorized_record(owner, task_id)?;
        record
            .lock()
            .map_err(|_| GraphicsTaskError::new("task_store_unavailable", "图形任务记录锁已损坏"))
            .map(|task| task.snapshot())
    }

    pub fn cancel(
        &self,
        owner: &GraphicsTaskOwner,
        task_id: &str,
    ) -> Result<GraphicsTaskSnapshot, GraphicsTaskError> {
        let record = self.authorized_record(owner, task_id)?;
        let mut task = record.lock().map_err(|_| {
            GraphicsTaskError::new("task_store_unavailable", "图形任务记录锁已损坏")
        })?;
        if !task.is_active() {
            return Err(GraphicsTaskError::new(
                "task_not_running",
                "图形任务已经结束",
            ));
        }
        let Some(cancel) = task.cancel.as_ref() else {
            return Err(GraphicsTaskError::new(
                "task_not_cancellable",
                "当前图形任务无法安全中断",
            ));
        };
        cancel.send(true).map_err(|_| {
            GraphicsTaskError::new("task_not_cancellable", "图形任务取消通道已关闭")
        })?;
        task.phase = "cancelling".to_string();
        task.message = "正在取消任务".to_string();
        Ok(task.snapshot())
    }

    fn authorized_record(
        &self,
        owner: &GraphicsTaskOwner,
        task_id: &str,
    ) -> Result<Arc<Mutex<GraphicsTaskRecord>>, GraphicsTaskError> {
        let record = self
            .tasks
            .lock()
            .map_err(|_| GraphicsTaskError::new("task_store_unavailable", "图形任务表锁已损坏"))?
            .get(task_id)
            .cloned()
            .ok_or_else(|| GraphicsTaskError::new("task_not_found", "图形任务不存在"))?;
        let task = record.lock().map_err(|_| {
            GraphicsTaskError::new("task_store_unavailable", "图形任务记录锁已损坏")
        })?;
        if task.owner.plugin_id != owner.plugin_id
            || task.owner.plugin_version != owner.plugin_version
            || task.owner.page_id != owner.page_id
            || task.owner.game_name != owner.game_name
        {
            return Err(GraphicsTaskError::new("task_not_found", "图形任务不存在"));
        }
        drop(task);
        Ok(record)
    }

    fn prune_finished(&self) {
        let Ok(tasks) = self.tasks.lock() else {
            return;
        };
        let now = unix_millis();
        let expired = tasks
            .iter()
            .filter_map(|(id, task)| {
                let task = task.lock().ok()?;
                if task.is_active() {
                    return None;
                }
                task.finished_at_ms
                    .is_some_and(|finished| {
                        now.saturating_sub(finished) > TASK_TTL.as_millis() as u64
                    })
                    .then_some(id.clone())
            })
            .collect::<Vec<_>>();
        drop(tasks);
        if let Ok(mut tasks) = self.tasks.lock() {
            for id in expired {
                tasks.remove(&id);
            }
        }
    }
}

async fn run_task(
    record: Arc<Mutex<GraphicsTaskRecord>>,
    job: GraphicsTaskJob,
    mut cancel_rx: watch::Receiver<bool>,
) {
    if let Ok(mut task) = record.lock() {
        task.mark_running();
    }
    let cancellable = job.cancellable();
    let task = record.clone();
    let operation = async move { execute_job(job, task).await };
    tokio::pin!(operation);
    if cancellable {
        tokio::select! {
            result = &mut operation => finish_from_result(&record, result),
            _ = wait_for_cancel(&mut cancel_rx) => {
                if let Ok(mut task) = record.lock() {
                    task.cancelled();
                }
            }
            _ = tokio::time::sleep(TASK_TIMEOUT) => {
                if let Ok(mut task) = record.lock() {
                    task.fail("图形任务超时，已停止继续等待".to_string());
                }
            }
        }
    } else {
        finish_from_result(&record, operation.await);
    }
}

async fn execute_job(
    job: GraphicsTaskJob,
    record: Arc<Mutex<GraphicsTaskRecord>>,
) -> Result<Value, String> {
    match job {
        GraphicsTaskJob::RuntimeDownload {
            runtime,
            route,
            profile,
        } => {
            let owner = task_owner(&record)?;
            match runtime {
                GraphicsRuntime::Gimi => {
                    let progress_record = record.clone();
                    let installation = download::download_gimi_hot_runtime_with_progress(
                        owner.game_name,
                        task_game_executable(&record)?,
                        profile,
                        move |progress: GraphicsProgress| {
                            report_progress(
                                &progress_record,
                                &progress.phase,
                                &progress.file,
                                progress.downloaded_bytes,
                                progress.total_bytes,
                            );
                        },
                    )
                    .await?;
                    serde_json::to_value(installation).map_err(|error| error.to_string())
                }
                GraphicsRuntime::Dlss5 => {
                    let progress_record = record.clone();
                    let installation = dlss5_hot_runtime::download_dlss5_hot_runtime_with_progress(
                        owner.game_name,
                        task_game_executable(&record)?,
                        profile,
                        route,
                        move |progress: GraphicsProgress| {
                            report_progress(
                                &progress_record,
                                &progress.phase,
                                &progress.file,
                                progress.downloaded_bytes,
                                progress.total_bytes,
                            );
                        },
                    )
                    .await?;
                    serde_json::to_value(installation).map_err(|error| error.to_string())
                }
            }
        }
        GraphicsTaskJob::RuntimeImport {
            source_directory,
            profile,
        } => {
            let owner = task_owner(&record)?;
            let game_executable = task_game_executable(&record)?;
            set_phase(&record, "importing", "正在导入并校验组件");
            let installation = tauri::async_runtime::spawn_blocking(move || {
                super::gimi_hot_runtime::import_gimi_hot_runtime(
                    owner.game_name,
                    game_executable,
                    source_directory,
                    profile,
                )
            })
            .await
            .map_err(|error| format!("后台导入任务失败：{error}"))??;
            serde_json::to_value(installation).map_err(|error| error.to_string())
        }
        GraphicsTaskJob::RuntimeSelect { root } => {
            let owner = task_owner(&record)?;
            let game_executable = task_game_executable(&record)?;
            set_phase(&record, "selecting", "正在校验并选择组件");
            tauri::async_runtime::spawn_blocking(move || {
                super::dlss5_hot_runtime::catalog::select_dlss5_hot_runtime(
                    owner.game_name,
                    game_executable,
                    root.into(),
                )
            })
            .await
            .map_err(|error| format!("后台选择任务失败：{error}"))??;
            Ok(json!({}))
        }
        GraphicsTaskJob::RuntimeRemove { root } => {
            let owner = task_owner(&record)?;
            let game_executable = task_game_executable(&record)?;
            set_phase(&record, "removing", "正在清理组件");
            tauri::async_runtime::spawn_blocking(move || {
                super::dlss5_hot_runtime::catalog::remove_dlss5_hot_runtime(
                    owner.game_name,
                    game_executable,
                    root.into(),
                )
            })
            .await
            .map_err(|error| format!("后台清理任务失败：{error}"))??;
            Ok(json!({}))
        }
        GraphicsTaskJob::RouteInstall {
            route,
            api_override,
            anti_cheat_acknowledged,
        } => {
            let owner = task_owner(&record)?;
            let game_executable = task_game_executable(&record)?;
            set_phase(&record, "installing", "正在安装兼容路线");
            let scan = tauri::async_runtime::spawn_blocking(move || {
                super::commands::install_managed_dlss5_route(
                    owner.game_name,
                    game_executable,
                    route,
                    api_override,
                    anti_cheat_acknowledged,
                )
            })
            .await
            .map_err(|error| format!("后台安装任务失败：{error}"))??;
            serde_json::to_value(scan).map_err(|error| error.to_string())
        }
        GraphicsTaskJob::RouteRestore => {
            let owner = task_owner(&record)?;
            let game_executable = task_game_executable(&record)?;
            set_phase(&record, "restoring", "正在恢复原安装");
            tauri::async_runtime::spawn_blocking(move || {
                super::commands::restore_dlss5_install(owner.game_name, game_executable)
            })
            .await
            .map_err(|error| format!("后台恢复任务失败：{error}"))??;
            Ok(json!({}))
        }
    }
}

fn finish_from_result(record: &Arc<Mutex<GraphicsTaskRecord>>, result: Result<Value, String>) {
    if let Ok(mut task) = record.lock() {
        match result {
            Ok(value) => task.succeed(value),
            Err(error) => task.fail(error),
        }
    }
}

fn report_progress(
    record: &Arc<Mutex<GraphicsTaskRecord>>,
    phase: &str,
    file: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
) {
    if let Ok(mut task) = record.lock() {
        task.report(GraphicsProgress {
            phase: phase.to_string(),
            file: file.to_string(),
            downloaded_bytes,
            total_bytes,
        });
    }
}

fn set_phase(record: &Arc<Mutex<GraphicsTaskRecord>>, phase: &str, message: &str) {
    if let Ok(mut task) = record.lock() {
        if task.is_active() {
            task.status = GraphicsTaskStatus::Running;
            task.phase = phase.to_string();
            task.message = message.to_string();
        }
    }
}

fn task_owner(record: &Arc<Mutex<GraphicsTaskRecord>>) -> Result<GraphicsTaskOwner, String> {
    record
        .lock()
        .map(|task| task.owner.clone())
        .map_err(|_| "图形任务记录锁已损坏".to_string())
}

fn task_game_executable(record: &Arc<Mutex<GraphicsTaskRecord>>) -> Result<String, String> {
    record
        .lock()
        .map(|task| task.game_executable.clone())
        .map_err(|_| "图形任务记录锁已损坏".to_string())
}

fn game_executable(game_name: &str) -> Result<String, String> {
    let path = crate::config::path_manager::PathManager::global_config_games_game_folder(game_name)
        .join("Config.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("无法读取游戏配置 {}：{error}", path.display()))?;
    let config = serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("无法解析游戏配置：{error}"))?;
    config
        .get("targetExePath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| "游戏配置缺少 targetExePath".to_string())
}

fn parse_runtime(payload: &Value) -> Result<GraphicsRuntime, GraphicsTaskError> {
    match required_string(payload, "runtime")? {
        "gimi" => Ok(GraphicsRuntime::Gimi),
        "dlss5" => Ok(GraphicsRuntime::Dlss5),
        value => Err(GraphicsTaskError::new(
            "invalid_payload",
            format!("不支持的图形运行时：{value}"),
        )),
    }
}

fn required_string<'a>(payload: &'a Value, key: &str) -> Result<&'a str, GraphicsTaskError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| GraphicsTaskError::new("invalid_payload", format!("缺少或为空的 {key}")))
}

fn optional_string<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    while cancel_rx.changed().await.is_ok() {
        if *cancel_rx.borrow() {
            return;
        }
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> GraphicsTaskOwner {
        GraphicsTaskOwner {
            plugin_id: "example.graphics".into(),
            plugin_version: "1.0.0".into(),
            game_name: "GIMI".into(),
            page_id: "settings".into(),
            service_key: String::new(),
        }
    }

    #[test]
    fn task_payloads_are_strict_and_only_downloads_are_cancellable() {
        let download = GraphicsTaskJob::parse(&json!({
            "operation": "runtime.download",
            "runtime": "dlss5",
            "profile": "rtx50",
            "route": "ssmt-feeder"
        }))
        .unwrap();
        assert!(download.cancellable());
        assert_eq!(download.operation(), "runtime.download");
        let restore = GraphicsTaskJob::parse(&json!({ "operation": "route.restore" })).unwrap();
        assert!(!restore.cancellable());
        assert!(GraphicsTaskJob::parse(&json!({ "operation": "runtime.download" })).is_err());
        assert!(GraphicsTaskJob::parse(&json!({ "operation": "unknown" })).is_err());
    }

    #[test]
    fn tasks_are_hidden_from_other_plugin_pages() {
        let broker = GraphicsTaskBroker::default();
        let record = Arc::new(Mutex::new(GraphicsTaskRecord {
            task_id: "task-1".into(),
            owner: owner(),
            game_executable: "C:/Game/YuanShen.exe".into(),
            operation: "route.restore".into(),
            status: GraphicsTaskStatus::Succeeded,
            phase: "completed".into(),
            message: String::new(),
            downloaded_bytes: 0,
            total_bytes: 0,
            cancellable: false,
            created_at_ms: 1,
            started_at_ms: Some(1),
            finished_at_ms: Some(2),
            result: None,
            error_code: None,
            error_message: None,
            cancel: None,
        }));
        broker.tasks.lock().unwrap().insert("task-1".into(), record);
        assert!(broker.status(&owner(), "task-1").is_ok());
        let mut foreign = owner();
        foreign.page_id = "other".into();
        assert_eq!(
            broker.status(&foreign, "task-1").unwrap_err().code,
            "task_not_found"
        );
    }
}
