//! Child processes the app starts (yt-dlp, a tool asked for its version): started from
//! an argument vector, in a process group of their own, and killed with everything under
//! them (`ChildProcessJob` and `Process.Kill(entireProcessTree: true)`).
//!
//! macOS has no job object that dies with its owner. What this does instead: every
//! child leads its own process group, a cancel kills the whole group (so the ffmpeg a
//! merging download started goes too), and an `atexit` hook kills every group still
//! running when the app exits normally, including through `process::exit`. A crash or a
//! SIGKILL of Modlight is the one case left: the children then run to their own end.

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

/// A command from an argument vector: no shell, its own process group, UTF-8 for Python
/// tools (yt-dlp reads its output encoding from the environment), stdin closed.
pub fn command(program: &std::path::Path, arguments: &[String]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
