//! Child processes the app starts (yt-dlp, a tool asked for its version): started from
//! an argument vector, in a process group of their own, and killed with everything under
//! them (`ChildProcessJob` and `Process.Kill(entireProcessTree: true)`).
//!
//! macOS has no job object that dies with its owner. What this does instead: every
//! child leads its own process group, a cancel kills the whole group (so the ffmpeg a
//! merging download started goes too), and an `atexit` hook kills every group still
//! running when the app exits normally, including through `process::exit`. A crash or a
//! SIGKILL of Modlight is the one case left: the children then run to their own end.
//!
//! Windows does what upstream's `ChildProcessJob` does: every child is put in a job
//! object of its own that kills what is in it when its last handle closes. A cancel
//! terminates the job (the child and everything it started since), and because the
//! handle belongs to this process, the children die with Modlight however it ends, a
//! crash included. Children start without a console window (`CREATE_NO_WINDOW`): the app
//! is a GUI program and a console tool would otherwise flash one up.

#[cfg(unix)]
pub use unix::{GroupGuard, kill_all, kill_group};
#[cfg(windows)]
pub use windows::{GroupGuard, kill_all};

/// `CREATE_NO_WINDOW`: a console program started by this GUI app gets no console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(unix)]
mod unix {
    use std::collections::HashSet;
    use std::sync::{Mutex, Once};

    static RUNNING: Mutex<Option<HashSet<i32>>> = Mutex::new(None);
    static HOOK: Once = Once::new();

    extern "C" fn kill_all_at_exit() {
        kill_all();
    }

    /// Kill every process group this app started and has not seen end.
    pub fn kill_all() {
        let groups: Vec<i32> = RUNNING
            .lock()
            .map(|set| {
                set.as_ref()
                    .map(|s| s.iter().copied().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        for group in groups {
            kill_group(group);
        }
    }

    /// SIGKILL to the whole group `group` leads.
    pub fn kill_group(group: i32) {
        if group > 0 {
            // SAFETY: plain syscall on a process group id we started.
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }

    /// A child's process group, registered until dropped.
    pub struct GroupGuard {
        group: i32,
    }

    impl GroupGuard {
        /// Remember `pid` (the leader of its own group) so it dies with the app.
        pub fn register(pid: u32) -> GroupGuard {
            HOOK.call_once(|| {
                // SAFETY: registering a plain extern "C" function.
                unsafe {
                    libc::atexit(kill_all_at_exit);
                }
            });
            let group = pid as i32;
            if let Ok(mut set) = RUNNING.lock() {
                set.get_or_insert_with(HashSet::new).insert(group);
            }
            GroupGuard { group }
        }

        pub fn group(&self) -> i32 {
            self.group
        }

        /// Kill the group now (cancel, pause, a deadline).
        pub fn kill(&self) {
            kill_group(self.group);
        }
    }

    impl Drop for GroupGuard {
        fn drop(&mut self) {
            if let Ok(mut set) = RUNNING.lock()
                && let Some(set) = set.as_mut()
            {
                set.remove(&self.group);
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// The jobs of the children still running, as handle values.
    static RUNNING: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

    /// Terminate every job this app made and has not seen end.
    pub fn kill_all() {
        let jobs: Vec<usize> = RUNNING
            .lock()
            .map(|set| {
                set.as_ref()
                    .map(|s| s.iter().copied().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        for job in jobs {
            // SAFETY: a job handle this module created and has not closed yet.
            unsafe {
                TerminateJobObject(job as HANDLE, 1);
            }
        }
    }

    /// A job that kills its processes when its handle closes, or `None`.
    fn new_job() -> Option<HANDLE> {
        // SAFETY: an unnamed job with default security; the limit structure is zeroed
        // and then filled with the one flag asked for.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return None;
            }
            Some(job)
        }
    }

    /// A child in a job of its own, registered until dropped. Dropping the guard closes
    /// the job, which ends whatever the child left running.
    pub struct GroupGuard {
        job: Option<usize>,
        pid: u32,
    }

    impl GroupGuard {
        /// Put `pid` (a child just started) in a job of its own.
        pub fn register(pid: u32) -> GroupGuard {
            let job = new_job().and_then(|job| {
                // SAFETY: opening a child of ours with the two rights a job needs; the
                // process handle is closed right after the assignment.
                unsafe {
                    let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                    let assigned =
                        !process.is_null() && AssignProcessToJobObject(job, process) != 0;
                    if !process.is_null() {
                        CloseHandle(process);
                    }
                    if assigned {
                        Some(job as usize)
                    } else {
                        log::warn!("process {pid} could not join a job object");
                        CloseHandle(job);
                        None
                    }
                }
            });
            if let Some(job) = job
                && let Ok(mut set) = RUNNING.lock()
            {
                set.get_or_insert_with(HashSet::new).insert(job);
            }
            GroupGuard { job, pid }
        }

        /// The child's process id.
        pub fn group(&self) -> i32 {
            self.pid as i32
        }

        /// Kill the child and everything it started (cancel, pause, a deadline).
        pub fn kill(&self) {
            match self.job {
                // SAFETY: a job handle this guard owns.
                Some(job) => unsafe {
                    TerminateJobObject(job as HANDLE, 1);
                },
                None => kill_process(self.pid),
            }
        }
    }

    /// The child alone, when it never made it into a job.
    fn kill_process(pid: u32) {
        use windows_sys::Win32::System::Threading::TerminateProcess;
        // SAFETY: open, terminate and close one process handle.
        unsafe {
            let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !process.is_null() {
                TerminateProcess(process, 1);
                CloseHandle(process);
            }
        }
    }

    impl Drop for GroupGuard {
        fn drop(&mut self) {
            if let Some(job) = self.job.take() {
                if let Ok(mut set) = RUNNING.lock()
                    && let Some(set) = set.as_mut()
                {
                    set.remove(&job);
                }
                // SAFETY: the guard's own job handle, closed exactly once.
                unsafe {
                    CloseHandle(job as HANDLE);
                }
            }
        }
    }
}

/// A command from an argument vector: no shell, its own process group, UTF-8 for Python
/// tools (yt-dlp reads its output encoding from the environment), stdin closed.
pub fn command(program: &std::path::Path, arguments: &[String]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// The same for a plain `std::process::Command` (ffmpeg, ExifTool, actions): on Windows
/// it starts without a console window.
pub fn no_window(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn a_killed_group_takes_its_children_along() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild.pid");
        // A shell that starts a sleeping grandchild and waits for it.
        let script = format!("sleep 30 & echo $! > '{}'; wait", marker.display());
        let mut child = command(std::path::Path::new("/bin/sh"), &["-c".to_string(), script])
            .spawn()
            .unwrap();
        let guard = GroupGuard::register(child.id().unwrap());
        for _ in 0..100 {
            if marker.exists() && !std::fs::read_to_string(&marker).unwrap().trim().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let grandchild: i32 = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        guard.kill();
        let _ = child.wait().await;
        // The grandchild is gone too (or a zombie being reaped): signal 0 fails soon.
        let mut gone = false;
        for _ in 0..100 {
            // SAFETY: probing a pid with signal 0.
            if unsafe { libc::kill(grandchild, 0) } != 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(gone, "the grandchild outlived its group");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_killed_job_takes_its_children_along() {
        // cmd starts a second cmd that waits; killing the job ends both.
        let mut child = command(
            std::path::Path::new("cmd.exe"),
            &[
                "/C".to_string(),
                "start /b cmd.exe /C ping -n 30 127.0.0.1 >NUL & ping -n 30 127.0.0.1 >NUL"
                    .to_string(),
            ],
        )
        .spawn()
        .unwrap();
        let guard = GroupGuard::register(child.id().unwrap());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        guard.kill();
        let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
            .await
            .expect("the child ended")
            .unwrap();
        assert!(!status.success());
    }
}
