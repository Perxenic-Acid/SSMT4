use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchBarrierError {
    UnsupportedPlatform,
    CreateFailed(String),
    WaitFailed(String),
    ArrivalTimeout,
    ReleaseFailed(String),
}

impl std::fmt::Display for LaunchBarrierError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform => write!(formatter, "launch barrier requires Windows"),
            Self::CreateFailed(error) => {
                write!(formatter, "failed to create launch barrier: {error}")
            }
            Self::WaitFailed(error) => {
                write!(formatter, "failed waiting for launch barrier: {error}")
            }
            Self::ArrivalTimeout => write!(
                formatter,
                "Run.exe did not reach before-resume barrier in time"
            ),
            Self::ReleaseFailed(error) => {
                write!(formatter, "failed to release launch barrier: {error}")
            }
        }
    }
}

impl std::error::Error for LaunchBarrierError {}

#[cfg(windows)]
mod platform {
    use super::{Duration, LaunchBarrierError};
    use std::ptr::null_mut;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

    pub struct LaunchBarrier {
        id: String,
        ready: HANDLE,
        release: HANDLE,
        released: bool,
    }

    impl LaunchBarrier {
        pub fn create() -> Result<Self, LaunchBarrierError> {
            let id = format!(
                "{:x}-{:x}-{:x}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default(),
                NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            );
            let ready_name = wide(&format!("Local\\SSMT4.Launch.{id}.Ready"));
            let release_name = wide(&format!("Local\\SSMT4.Launch.{id}.Release"));
            let ready = unsafe { CreateEventW(null_mut(), 1, 0, ready_name.as_ptr()) };
            if ready.is_null() {
                return Err(LaunchBarrierError::CreateFailed(last_error()));
            }
            let release = unsafe { CreateEventW(null_mut(), 1, 0, release_name.as_ptr()) };
            if release.is_null() {
                unsafe { CloseHandle(ready) };
                return Err(LaunchBarrierError::CreateFailed(last_error()));
            }
            Ok(Self {
                id,
                ready,
                release,
                released: false,
            })
        }

        pub fn id(&self) -> &str {
            &self.id
        }

        pub fn wait_until_ready(&self, timeout: Duration) -> Result<(), LaunchBarrierError> {
            let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
            match unsafe { WaitForSingleObject(self.ready, timeout_ms) } {
                WAIT_OBJECT_0 => Ok(()),
                WAIT_TIMEOUT => Err(LaunchBarrierError::ArrivalTimeout),
                _ => Err(LaunchBarrierError::WaitFailed(last_error())),
            }
        }

        pub fn release(&mut self) -> Result<(), LaunchBarrierError> {
            if unsafe { SetEvent(self.release) } == 0 {
                return Err(LaunchBarrierError::ReleaseFailed(last_error()));
            }
            self.released = true;
            Ok(())
        }

        pub fn run_argument(&self) -> String {
            format!("--launch-barrier {}", self.id)
        }
    }

    impl Drop for LaunchBarrier {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.ready);
                CloseHandle(self.release);
            }
        }
    }

    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> String {
        format!("Windows error {}", unsafe { GetLastError() })
    }
}

#[cfg(windows)]
pub use platform::LaunchBarrier;

#[cfg(not(windows))]
pub struct LaunchBarrier;

#[cfg(not(windows))]
impl LaunchBarrier {
    pub fn create() -> Result<Self, LaunchBarrierError> {
        Err(LaunchBarrierError::UnsupportedPlatform)
    }

    pub fn id(&self) -> &str {
        ""
    }

    pub fn wait_until_ready(&self, _timeout: Duration) -> Result<(), LaunchBarrierError> {
        Err(LaunchBarrierError::UnsupportedPlatform)
    }

    pub fn release(&mut self) -> Result<(), LaunchBarrierError> {
        Err(LaunchBarrierError::UnsupportedPlatform)
    }

    pub fn run_argument(&self) -> String {
        String::new()
    }
}
