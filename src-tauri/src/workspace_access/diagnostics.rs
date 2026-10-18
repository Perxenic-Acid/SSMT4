//! 每次发布保留阶段与错误；不记录请求正文、凭据或 R2 签名地址。
use super::models::{
    WorkspaceAccessPublishRequest, WorkspacePublishResult, WorkspaceUploadProgress,
};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePublishError {
    pub message: String,
    pub stage: &'static str,
    pub log_path: Option<String>,
}

pub struct PublishLog {
    file: Option<File>,
    path: Option<PathBuf>,
    stage: &'static str,
}

impl PublishLog {
    pub fn new(folder: &Path, request: &WorkspaceAccessPublishRequest) -> Self {
        let path = folder.join(format!(
            "{}-workspace-publish-{}.log",
            chrono::Local::now().format("%Y%m%dT%H%M%S%.9f"),
            std::process::id()
        ));
        let file = fs::create_dir_all(folder)
            .and_then(|_| File::options().write(true).create_new(true).open(&path));
        let mut log = match file {
            Ok(file) => Self {
                file: Some(file),
                path: Some(path),
                stage: "initialize",
            },
            Err(error) => {
                eprintln!("[WorkspacePublish] 无法创建日志：{error}");
                Self {
                    file: None,
                    path: None,
                    stage: "initialize",
                }
            }
        };
        log.write(&format!(
            "游戏={}；工作空间={}；目录={}；代理端口={:?}",
            request.game_preset, request.workspace_name, request.workspace_path, request.proxy_port
        ));
        log
    }

    pub fn stage(&mut self, stage: &'static str) {
        self.stage = stage;
        self.write("开始");
    }

    pub fn progress(&mut self, progress: &WorkspaceUploadProgress) {
        self.stage = if progress.completed_bytes == progress.total_bytes {
            "finalize"
        } else {
            "upload"
        };
        self.write(&format!(
            "submission={}；字节={}/{}；分片={}/{}",
            progress.submission_id,
            progress.completed_bytes,
            progress.total_bytes,
            progress.completed_parts,
            progress.total_parts
        ));
    }

    pub fn write(&mut self, message: &str) {
        if let Some(file) = self.file.as_mut() {
            let message: String = message
                .chars()
                .take(4096)
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect();
            if let Err(error) = writeln!(
                file,
                "{} [{}] {}",
                chrono::Local::now().format("%H:%M:%S%.3f"),
                self.stage,
                message
            ) {
                eprintln!("[WorkspacePublish] 日志写入失败：{error}");
                self.file = None;
            }
        }
    }

    pub fn finish(
        &mut self,
        result: Result<WorkspacePublishResult, String>,
    ) -> Result<WorkspacePublishResult, WorkspacePublishError> {
        match result {
            Ok(value) => {
                self.write(&format!(
                    "发布成功：entry={}；status={}",
                    value.entry_id, value.status
                ));
                Ok(value)
            }
            Err(message) => {
                self.write(&format!("失败：{message}"));
                Err(WorkspacePublishError {
                    message,
                    stage: self.stage,
                    log_path: self
                        .path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                })
            }
        }
    }
}
