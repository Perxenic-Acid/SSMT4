use std::io;
use std::process::{Child, Command, Output};

// 只用于交接给用户的游戏启动链及其清理 helper；普通短期工具仍由调用方管理。
// CREATE_NO_WINDOW / DETACHED_PROCESS 仅隔离控制台，并不能脱离 Cargo 等上级的 Job。
fn launch<T>(command: &mut Command, run: impl Fn(&mut Command) -> io::Result<T>) -> io::Result<T> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW};

        command.creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        match run(command) {
            Err(error) if error.raw_os_error() == Some(5) => {
                // 有些受管环境禁止脱离 Job。创建尚未成功，才可按原有方式重试；
                // 不以生命周期隔离失败阻断游戏，也不能把降级伪称为已经独立。
                eprintln!("[GameLauncher] 无法脱离上级进程的 Job（访问被拒绝），沿用原启动方式；上级退出时仍可能结束游戏。");
                command.creation_flags(CREATE_NO_WINDOW);
            }
            result => return result,
        }
    }
    run(command)
}

pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    launch(command, Command::spawn)
}

pub(crate) fn output(command: &mut Command) -> io::Result<Output> {
    launch(command, Command::output)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, CREATE_NO_WINDOW};

    fn fixture_command(mode: &str, marker: &std::path::Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let (_, module) = module_path!().split_once("::").unwrap();
        command
            .args([
                "--exact",
                &format!("{module}::lifetime_fixture"),
                "--ignored",
                "--nocapture",
            ])
            .env("SSMT_LIFETIME_FIXTURE_MODE", mode)
            .env("SSMT_LIFETIME_FIXTURE_MARKER", marker)
            .creation_flags(CREATE_NO_WINDOW);
        command
    }

    fn marker(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ssmt-lifetime-{}-{}-{label}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn parent_fixture(mode: &str, marker: &std::path::Path) -> Output {
        let result = fixture_command(mode, marker).output().unwrap();
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("fixture-spawned"),
            "夹具未创建子进程：{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result
    }

    fn wait_for_marker(marker: &std::path::Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "survived");
        std::fs::remove_file(marker).unwrap();
    }

    #[test]
    fn launch_chain_survives_parent_job_closing_while_original_path_does_not() {
        let inherited = marker("inherited");
        let independent = marker("independent");
        parent_fixture("inherited", &inherited);
        parent_fixture("independent", &independent);
        wait_for_marker(&independent);
        assert!(!inherited.exists(), "对照路径未复现 Job 结束子进程");
    }

    #[test]
    fn restricted_job_falls_back_without_preventing_launch() {
        let marker = marker("restricted");
        let result = parent_fixture("restricted", &marker);
        assert!(String::from_utf8_lossy(&result.stderr).contains("无法脱离上级进程的 Job"));
        wait_for_marker(&marker);
    }

    #[test]
    fn missing_program_still_reports_spawn_error() {
        assert_eq!(
            spawn(&mut Command::new(marker("missing")))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    #[ignore = "仅由生命周期回归在隔离子进程中调用，不接触游戏"]
    fn lifetime_fixture() {
        let mode = std::env::var("SSMT_LIFETIME_FIXTURE_MODE").unwrap();
        let marker = PathBuf::from(std::env::var_os("SSMT_LIFETIME_FIXTURE_MARKER").unwrap());
        if mode == "payload" {
            std::thread::sleep(Duration::from_millis(400));
            std::fs::write(marker, "survived").unwrap();
            return;
        }

        // 只将本次夹具进程放入自建 Job。关闭它会结束本夹具和仍继承该 Job 的
        // 后代；测试控制进程及用户程序均不在此 Job 内。
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            assert!(!job.is_null(), "{}", io::Error::last_os_error());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if mode != "restricted" {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            }
            assert_ne!(
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    std::mem::size_of_val(&info) as u32
                ),
                0,
                "{}",
                io::Error::last_os_error()
            );
            assert_ne!(
                AssignProcessToJobObject(job, GetCurrentProcess()),
                0,
                "{}",
                io::Error::last_os_error()
            );
            let mut command = fixture_command("payload", &marker);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let mut child = if mode == "inherited" {
                command.spawn()
            } else {
                spawn(&mut command)
            }
            .unwrap();
            println!("fixture-spawned");
            std::io::stdout().flush().unwrap();
            if mode == "restricted" {
                assert!(child.wait().unwrap().success());
            }
            CloseHandle(job);
            // KILL_ON_JOB_CLOSE 连同本夹具一起结束，不能继续执行普通测试。
            std::process::abort();
        }
    }
}
