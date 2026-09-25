use super::registry::InstalledPlugin;
use super::{
    FailurePolicy, LauncherAdapterContribution, LauncherAdapterType, ReadyCondition,
    ReadyConditionType,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const MAX_CAPTURED_BYTES: usize = 256 * 1024;
const OUTPUT_CHANNEL_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTemplateContext {
    pub game_executable: PathBuf,
    pub game_directory: PathBuf,
    pub game_process_name: String,
    pub external_dependencies: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessForwarderSpec {
    pub plugin_id: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub timeout: Option<Duration>,
    pub ready: Option<ReadyCondition>,
    pub failure_policy: FailurePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwarderOutput {
    pub stream: OutputStream,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapturedOutput {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct ProcessForwarderResult {
    pub status: ExitStatus,
    pub output: CapturedOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessForwarderError {
    UnsupportedAdapterType,
    InvalidTemplate(String),
    MissingExternalDependency(String),
    InvalidExecutable(PathBuf),
    InvalidWorkingDirectory(PathBuf),
    SpawnFailed(String),
    OutputReaderFailed(String),
    TimedOut,
    ReadyTimeout,
    ExitedBeforeReady(Option<i32>),
}

impl std::fmt::Display for ProcessForwarderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedAdapterType => write!(formatter, "unsupported launcher adapter type"),
            Self::InvalidTemplate(value) => write!(formatter, "invalid launcher template: {value}"),
            Self::MissingExternalDependency(id) => {
                write!(formatter, "external dependency is not ready: {id}")
            }
            Self::InvalidExecutable(path) => {
                write!(
                    formatter,
                    "launcher executable is not a file: {}",
                    path.display()
                )
            }
            Self::InvalidWorkingDirectory(path) => write!(
                formatter,
                "launcher working directory is not a directory: {}",
                path.display()
            ),
            Self::SpawnFailed(error) => write!(formatter, "failed to start launcher: {error}"),
            Self::OutputReaderFailed(error) => {
                write!(formatter, "failed to capture launcher output: {error}")
            }
            Self::TimedOut => write!(formatter, "launcher process timed out"),
            Self::ReadyTimeout => write!(formatter, "launcher ready condition timed out"),
            Self::ExitedBeforeReady(code) => {
                write!(formatter, "launcher exited before ready, code={code:?}")
            }
        }
    }
}

impl std::error::Error for ProcessForwarderError {}

impl ProcessForwarderSpec {
    pub fn resolve(
        plugin: &InstalledPlugin,
        contribution: &LauncherAdapterContribution,
        context: &LaunchTemplateContext,
        timeout: Option<Duration>,
    ) -> Result<Self, ProcessForwarderError> {
        if contribution.kind != LauncherAdapterType::ProcessForwarder {
            return Err(ProcessForwarderError::UnsupportedAdapterType);
        }

        let executable = resolve_template(&contribution.executable, plugin, context)?;
        let working_directory = match contribution.working_directory.as_deref() {
            Some(value) => resolve_template(value, plugin, context)?,
            None => executable
                .parent()
                .map(PathBuf::from)
                .ok_or_else(|| ProcessForwarderError::InvalidTemplate(value_path(&executable)))?,
        };
        let arguments = contribution
            .arguments
            .iter()
            .map(|argument| resolve_argument(argument, plugin, context))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            plugin_id: plugin.manifest.id.clone(),
            executable,
            arguments,
            working_directory,
            timeout,
            ready: contribution.ready.clone(),
            failure_policy: contribution.failure_policy,
        })
    }

    pub async fn run<F>(
        self,
        mut on_output: F,
    ) -> Result<ProcessForwarderResult, ProcessForwarderError>
    where
        F: FnMut(ForwarderOutput) + Send,
    {
        if !self.executable.is_file() {
            return Err(ProcessForwarderError::InvalidExecutable(self.executable));
        }
        if !self.working_directory.is_dir() {
            return Err(ProcessForwarderError::InvalidWorkingDirectory(
                self.working_directory,
            ));
        }

        let mut command = Command::new(&self.executable);
        command
            .args(&self.arguments)
            .current_dir(&self.working_directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| ProcessForwarderError::SpawnFailed(error.to_string()))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ProcessForwarderError::SpawnFailed("stdout pipe unavailable".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ProcessForwarderError::SpawnFailed("stderr pipe unavailable".to_string())
        })?;

        let (sender, mut receiver) = mpsc::channel(OUTPUT_CHANNEL_CAPACITY);
        let stdout_task = read_output(stdout, OutputStream::Stdout, sender.clone());
        let stderr_task = read_output(stderr, OutputStream::Stderr, sender.clone());
        drop(sender);
        let mut output = CapturedOutput::default();

        if let Some(ready) = self.ready.as_ref() {
            let ready_result = wait_for_ready(
                &mut child,
                &mut receiver,
                &mut on_output,
                &mut output,
                ready,
                self.failure_policy,
            )
            .await?;
            if let Err(ProcessForwarderError::ExitedBeforeReady(code)) = ready_result {
                match self.failure_policy {
                    FailurePolicy::Abort => {
                        return Err(ProcessForwarderError::ExitedBeforeReady(code));
                    }
                    FailurePolicy::Warn | FailurePolicy::Ignore => {}
                }
            }
        }

        let wait = collect_until_exit(
            &mut child,
            &mut receiver,
            &mut on_output,
            output,
            stdout_task,
            stderr_task,
        );
        match self.timeout {
            Some(timeout) => tokio::time::timeout(timeout, wait)
                .await
                .map_err(|_| ProcessForwarderError::TimedOut)?,
            None => wait.await,
        }
    }
}

fn read_output<R>(
    reader: R,
    stream: OutputStream,
    sender: mpsc::Sender<ForwarderOutput>,
) -> JoinHandle<Result<(), std::io::Error>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await? {
            if sender.send(ForwarderOutput { stream, line }).await.is_err() {
                break;
            }
        }
        Ok(())
    })
}

async fn collect_until_exit<F>(
    child: &mut Child,
    receiver: &mut mpsc::Receiver<ForwarderOutput>,
    on_output: &mut F,
    mut output: CapturedOutput,
    stdout_task: JoinHandle<Result<(), std::io::Error>>,
    stderr_task: JoinHandle<Result<(), std::io::Error>>,
) -> Result<ProcessForwarderResult, ProcessForwarderError>
where
    F: FnMut(ForwarderOutput),
{
    let status = loop {
        tokio::select! {
            status = child.wait() => {
                break status.map_err(|error| ProcessForwarderError::SpawnFailed(error.to_string()))?;
            }
            line = receiver.recv() => {
                if let Some(line) = line {
                    append_output(&mut output, &line);
                    on_output(line);
                }
            }
        }
    };

    while let Some(line) = receiver.recv().await {
        append_output(&mut output, &line);
        on_output(line);
    }

    await_reader(stdout_task).await?;
    await_reader(stderr_task).await?;
    Ok(ProcessForwarderResult { status, output })
}

async fn wait_for_ready<F>(
    child: &mut Child,
    receiver: &mut mpsc::Receiver<ForwarderOutput>,
    on_output: &mut F,
    output: &mut CapturedOutput,
    ready: &ReadyCondition,
    failure_policy: FailurePolicy,
) -> Result<Result<(), ProcessForwarderError>, ProcessForwarderError>
where
    F: FnMut(ForwarderOutput),
{
    let wait = async {
        loop {
            tokio::select! {
                status = child.wait() => {
                    let status = status.map_err(|error| ProcessForwarderError::SpawnFailed(error.to_string()))?;
                    if matches!(ready.kind, ReadyConditionType::ProcessExited) {
                        return Ok(Ok(()));
                    }
                    return Ok(Err(ProcessForwarderError::ExitedBeforeReady(status.code())));
                }
                line = receiver.recv() => {
                    let Some(line) = line else {
                        continue;
                    };
                    append_output(output, &line);
                    let matched = matches!((ready.kind, line.stream),
                        (ReadyConditionType::StdoutContains, OutputStream::Stdout) |
                        (ReadyConditionType::StderrContains, OutputStream::Stderr))
                        && ready.value.as_deref().is_some_and(|value| line.line.contains(value));
                    on_output(line);
                    if matched {
                        return Ok(Ok(()));
                    }
                }
            }
        }
    };

    match tokio::time::timeout(Duration::from_millis(ready.timeout_ms), wait).await {
        Ok(result) => result,
        Err(_) => match failure_policy {
            FailurePolicy::Abort => Err(ProcessForwarderError::ReadyTimeout),
            FailurePolicy::Warn | FailurePolicy::Ignore => Ok(Ok(())),
        },
    }
}

async fn await_reader(
    task: JoinHandle<Result<(), std::io::Error>>,
) -> Result<(), ProcessForwarderError> {
    task.await
        .map_err(|error| ProcessForwarderError::OutputReaderFailed(error.to_string()))?
        .map_err(|error| ProcessForwarderError::OutputReaderFailed(error.to_string()))
}

fn append_output(output: &mut CapturedOutput, line: &ForwarderOutput) {
    let target = match line.stream {
        OutputStream::Stdout => &mut output.stdout,
        OutputStream::Stderr => &mut output.stderr,
    };
    let remaining = MAX_CAPTURED_BYTES.saturating_sub(target.len());
    if remaining == 0 {
        output.truncated = true;
        return;
    }
    let line_bytes = line.line.as_bytes();
    let copied = remaining.min(line_bytes.len());
    target.push_str(&String::from_utf8_lossy(&line_bytes[..copied]));
    target.push('\n');
    if copied < line_bytes.len() {
        output.truncated = true;
    }
}

fn resolve_template(
    value: &str,
    plugin: &InstalledPlugin,
    context: &LaunchTemplateContext,
) -> Result<PathBuf, ProcessForwarderError> {
    Ok(PathBuf::from(resolve_argument(value, plugin, context)?))
}

fn resolve_argument(
    value: &str,
    plugin: &InstalledPlugin,
    context: &LaunchTemplateContext,
) -> Result<String, ProcessForwarderError> {
    let mut resolved = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("${") {
        resolved.push_str(&remaining[..start]);
        let token_start = start + 2;
        let Some(end) = remaining[token_start..].find('}') else {
            return Err(ProcessForwarderError::InvalidTemplate(value.to_string()));
        };
        let variable = &remaining[token_start..token_start + end];
        let replacement = match variable {
            "game.exe" => context.game_executable.to_string_lossy().into_owned(),
            "game.directory" => context.game_directory.to_string_lossy().into_owned(),
            "game.processName" => context.game_process_name.clone(),
            "plugin.root" => plugin.package_root.to_string_lossy().into_owned(),
            _ => {
                let dependency_id = variable
                    .strip_prefix("external.")
                    .ok_or_else(|| ProcessForwarderError::InvalidTemplate(value.to_string()))?;
                context
                    .external_dependencies
                    .get(dependency_id)
                    .ok_or_else(|| {
                        ProcessForwarderError::MissingExternalDependency(dependency_id.to_string())
                    })?
                    .to_string_lossy()
                    .into_owned()
            }
        };
        resolved.push_str(&replacement);
        remaining = &remaining[token_start + end + 1..];
    }
    if remaining.contains('}') {
        return Err(ProcessForwarderError::InvalidTemplate(value.to_string()));
    }
    resolved.push_str(remaining);
    Ok(resolved)
}

fn value_path(path: &std::path::Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::registry::{
        ExternalDependencyState, ExternalDependencyStatus, InstalledPlugin,
    };
    use crate::plugin::{
        ExternalDependency, ExternalDependencyType, PluginCompatibility, PluginContributions,
        PluginManifest, PluginPermission,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ssmt-process-forwarder-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn plugin(root: &std::path::Path) -> InstalledPlugin {
        InstalledPlugin {
            manifest: PluginManifest {
                schema_version: 1,
                id: "ssmt.test.forwarder".to_string(),
                name: "Test Forwarder".to_string(),
                version: "1.0.0".to_string(),
                author: "SSMT".to_string(),
                compatibility: PluginCompatibility {
                    ssmt: ">=4.0.0".to_string(),
                    platforms: vec!["windows-x64".to_string()],
                },
                contributions: PluginContributions::default(),
                external_dependencies: vec![ExternalDependency {
                    id: "tool".to_string(),
                    kind: ExternalDependencyType::Directory,
                    required_files: Vec::new(),
                }],
                permissions: vec![PluginPermission::ProcessSpawn],
            },
            package_root: root.join("package"),
            enabled: true,
            external_dependencies: BTreeMap::from([(
                "tool".to_string(),
                ExternalDependencyState {
                    status: ExternalDependencyStatus::Ready,
                    path: Some(root.join("external")),
                    reason: None,
                },
            )]),
        }
    }

    fn context(root: &std::path::Path) -> LaunchTemplateContext {
        LaunchTemplateContext {
            game_executable: root.join("Game.exe"),
            game_directory: root.to_path_buf(),
            game_process_name: "Game.exe".to_string(),
            external_dependencies: BTreeMap::from([("tool".to_string(), root.join("external"))]),
        }
    }

    #[test]
    fn interpolates_supported_variables_and_keeps_arguments_separate() {
        let root = temp_root("interpolation");
        let plugin = plugin(&root);
        let context = context(&root);
        let adapter = LauncherAdapterContribution {
            kind: LauncherAdapterType::ProcessForwarder,
            executable: "${external.tool}/inject.exe".to_string(),
            arguments: vec![
                "${game.processName}".to_string(),
                "--path=${game.directory}".to_string(),
            ],
            working_directory: Some("${plugin.root}".to_string()),
            ready: None,
            failure_policy: FailurePolicy::Abort,
        };
        let spec = ProcessForwarderSpec::resolve(&plugin, &adapter, &context, None).unwrap();
        assert_eq!(spec.executable, root.join("external/inject.exe"));
        assert_eq!(
            spec.arguments,
            vec!["Game.exe".to_string(), format!("--path={}", root.display())]
        );
        assert_eq!(spec.working_directory, root.join("package"));
    }

    #[test]
    fn rejects_unknown_or_unclosed_template_variables() {
        let root = temp_root("bad-template");
        let plugin = plugin(&root);
        let context = context(&root);
        for executable in ["${unknown}/tool.exe", "${external.tool/tool.exe"] {
            let adapter = LauncherAdapterContribution {
                kind: LauncherAdapterType::ProcessForwarder,
                executable: executable.to_string(),
                arguments: Vec::new(),
                working_directory: None,
                ready: None,
                failure_policy: FailurePolicy::Abort,
            };
            assert!(ProcessForwarderSpec::resolve(&plugin, &adapter, &context, None).is_err());
        }
    }

    #[tokio::test]
    async fn captures_both_streams_and_reports_exit_status() {
        let root = temp_root("capture");
        fs::create_dir_all(&root).unwrap();
        let executable = std::env::current_exe().unwrap();
        let spec = ProcessForwarderSpec {
            plugin_id: "test".to_string(),
            executable: executable.clone(),
            arguments: vec![
                "--exact".to_string(),
                "plugin::process_forwarder::tests::forwarder_child_process".to_string(),
                "--nocapture".to_string(),
            ],
            working_directory: root.clone(),
            timeout: Some(Duration::from_secs(20)),
            ready: None,
            failure_policy: FailurePolicy::Abort,
        };
        let mut command = spec;
        command.arguments = vec![
            "--exact".to_string(),
            "plugin::process_forwarder::tests::forwarder_child_process".to_string(),
            "--nocapture".to_string(),
        ];
        let result = command.run(|_| {}).await.unwrap();
        assert!(result.status.success());
        assert!(result.output.stdout.contains("forwarder-child-stdout"));
        assert!(result.output.stderr.contains("forwarder-child-stderr"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn waits_for_stderr_ready_marker_before_finishing() {
        let root = temp_root("ready");
        fs::create_dir_all(&root).unwrap();
        let spec = ProcessForwarderSpec {
            plugin_id: "test".to_string(),
            executable: std::env::current_exe().unwrap(),
            arguments: vec![
                "--exact".to_string(),
                "plugin::process_forwarder::tests::forwarder_child_process".to_string(),
                "--nocapture".to_string(),
            ],
            working_directory: root.clone(),
            timeout: Some(Duration::from_secs(20)),
            ready: Some(ReadyCondition {
                kind: ReadyConditionType::StderrContains,
                value: Some("forwarder-child-stderr".to_string()),
                timeout_ms: 5_000,
            }),
            failure_policy: FailurePolicy::Abort,
        };
        let result = spec.run(|_| {}).await.unwrap();
        assert!(result.status.success());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ready_timeout_obeys_failure_policy() {
        let root = temp_root("ready-policy");
        fs::create_dir_all(&root).unwrap();
        let base = || ProcessForwarderSpec {
            plugin_id: "test".to_string(),
            executable: std::env::current_exe().unwrap(),
            arguments: vec![
                "--exact".to_string(),
                "plugin::process_forwarder::tests::forwarder_child_process".to_string(),
                "--nocapture".to_string(),
            ],
            working_directory: root.clone(),
            timeout: Some(Duration::from_secs(20)),
            ready: Some(ReadyCondition {
                kind: ReadyConditionType::StdoutContains,
                value: Some("missing-ready-marker".to_string()),
                timeout_ms: 1,
            }),
            failure_policy: FailurePolicy::Abort,
        };

        assert!(matches!(
            base().run(|_| {}).await,
            Err(ProcessForwarderError::ReadyTimeout)
        ));
        let mut warn = base();
        warn.failure_policy = FailurePolicy::Warn;
        assert!(warn.run(|_| {}).await.unwrap().status.success());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forwarder_child_process() {
        println!("forwarder-child-stdout");
        eprintln!("forwarder-child-stderr");
    }
}
