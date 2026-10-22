use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub game_name: String,
    pub directory: String,
    pub operation: String,
    pub target_version: String,
    pub state: String,
    pub phase: String,
    pub message: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
}
pub struct Task {
    pub snapshot: Mutex<Snapshot>,
    pub cancel: AtomicBool,
    pub cache_locks: Vec<Mutex<()>>,
}
static CURRENT: OnceLock<Mutex<Option<Arc<Task>>>> = OnceLock::new();
fn registry() -> &'static Mutex<Option<Arc<Task>>> {
    CURRENT.get_or_init(|| Mutex::new(None))
}
impl Task {
    pub fn new(game_name: String, directory: String) -> Arc<Self> {
        Arc::new(Self {
            cancel: AtomicBool::new(false),
            cache_locks: (0..64).map(|_| Mutex::new(())).collect(),
            snapshot: Mutex::new(Snapshot {
                id: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_string(),
                game_name,
                directory,
                operation: String::new(),
                target_version: String::new(),
                state: "running".into(),
                phase: "query".into(),
                message: "查询官方资源".into(),
                bytes_done: 0,
                bytes_total: 0,
                files_done: 0,
                files_total: 0,
            }),
        })
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("任务已暂停，已校验缓存可在下次继续使用".into())
        } else {
            Ok(())
        }
    }
    pub fn phase(&self, phase: &str, message: impl Into<String>) {
        let mut s = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        s.phase = phase.into();
        s.message = message.into();
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn finish(&self, result: Result<(), String>) {
        let mut s = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(()) => {
                s.state = "completed".into();
                s.message = "任务完成".into();
            }
            Err(e) => {
                s.state = if self.cancel.load(Ordering::Relaxed) {
                    "paused"
                } else {
                    "failed"
                }
                .into();
                s.message = e;
            }
        }
    }
}
pub fn reserve(task: Arc<Task>) -> Result<(), String> {
    let mut current = registry().lock().map_err(|e| e.to_string())?;
    if current
        .as_ref()
        .is_some_and(|t| t.snapshot().state == "running")
    {
        return Err("已有游戏维护任务，请等待完成或暂停后再开始".into());
    }
    *current = Some(task);
    Ok(())
}
pub fn current() -> Option<Arc<Task>> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn assert_launch_allowed(game_name: Option<&str>) -> Result<(), String> {
    if current().is_some_and(|t| {
        let s = t.snapshot();
        s.state == "running"
            && s.operation != "preload"
            && game_name.map_or(s.phase == "commit", |name| s.game_name == name)
    }) {
        return Err("游戏客户端正在维护，请完成或暂停后再启动".into());
    }
    Ok(())
}

#[tauri::command]
pub fn hoyoplay_task() -> Option<Snapshot> {
    current().map(|t| t.snapshot())
}
#[tauri::command]
pub fn hoyoplay_cancel() -> Result<(), String> {
    if let Some(t) = current() {
        if t.snapshot().phase == "commit" {
            return Err("正在提交客户端文件，完成后才可暂停".into());
        }
        t.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}
