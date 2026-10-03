//! Process ownership and cooperative cancellation.
use super::{
    agents::{resolve_command, AgentPreset},
    AcpError,
};
use std::{process::Stdio, sync::Arc};
use tokio::{
    process::{Child, Command},
    sync::watch,
};

#[derive(Debug, Clone)]
pub struct SessionHandle {
    inner: Arc<Control>,
}
#[derive(Debug)]
struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
impl SessionHandle {
    pub fn channel() -> (Self, watch::Receiver<bool>, watch::Sender<bool>) {
        let (cancel, rx) = watch::channel(false);
        let (done, finished) = watch::channel(false);
        (
            Self {
                inner: Arc::new(Control {
                    cancel,
                    done: finished,
                }),
            },
            rx,
            done,
        )
    }
    pub async fn cancel(&self) {
        self.inner.cancel.send_replace(true);
        self.wait().await;
    }
    pub async fn wait(&self) {
        let mut done = self.inner.done.clone();
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}

pub async fn spawn_agent(preset: &AgentPreset, cwd: &std::path::Path) -> Result<Child, AcpError> {
    spawn_agent_with_policy(preset, cwd, false).await
}
pub async fn spawn_agent_with_policy(
    preset: &AgentPreset,
    cwd: &std::path::Path,
    modification: bool,
) -> Result<Child, AcpError> {
    let (program, args) = resolve_command(&preset.command, &preset.args)
        .ok_or_else(|| AcpError::AgentNotInstalled(preset.command.clone()))?;
    let mut cmd = Command::new(program);
    if modification && preset.id == "codex" {
        // Do not inherit full-access/auto-review mode from the user's adapter environment.
        cmd.env("INITIAL_AGENT_MODE", "read-only");
        cmd.env(
            "CODEX_CONFIG",
            r#"{"features":{"shell_tool":false},"web_search":"disabled"}"#,
        );
    }
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    #[cfg(unix)]
    cmd.process_group(0);
    Ok(cmd.spawn()?)
}

/// Windows job membership makes adapter descendants part of the same lifetime.
#[cfg(windows)]
pub struct ProcessTree(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
impl ProcessTree {
    pub fn attach(child: &Child) -> std::io::Result<Self> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            );
            if ok == 0 || AssignProcessToJobObject(job, child.raw_handle().unwrap() as _) == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self(job))
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(unix)]
pub struct ProcessTree(libc::pid_t);
#[cfg(unix)]
impl ProcessTree {
    pub fn attach(child: &Child) -> std::io::Result<Self> {
        let id = child.id().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "adapter has exited")
        })?;
        // The adapter becomes group leader at spawn, before it can start descendants.
        Ok(Self(id as libc::pid_t))
    }
}
#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // SAFETY: a negative PID targets only this session's process group.
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}
#[cfg(not(any(windows, unix)))]
pub struct ProcessTree;
#[cfg(not(any(windows, unix)))]
impl ProcessTree {
    pub fn attach(_: &Child) -> std::io::Result<Self> {
        Ok(Self)
    }
}
