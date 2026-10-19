//! Port of `lib/media/ffmpeg.ts`: running ffmpeg on files other people
//! uploaded.
//!
//! Everything here exists because the input is hostile. ffmpeg is a demuxer zoo
//! with a URL parser bolted on, and its defaults are tuned for a person at a
//! terminal converting their own holiday video, not for a public HTTP endpoint on
//! a box shared with sixteen other stacks.
//!
//! The four defaults that had to be reversed, each verified against ffmpeg 5.1.9
//! in the runtime image rather than assumed:
//!
//!  1. PROTOCOLS. `-protocol_whitelist file,pipe` - anything else and a crafted
//!     file can make ffmpeg fetch a URL for us. Measured: `-i http://…` is
//!     refused with "Protocol 'http' not on whitelist", by ffmpeg and ffprobe
//!     alike.
//!
//!  2. DEMUXER AUTODETECTION. This is the one that actually bites. Handed a text
//!     file containing `ffconcat version 1.0 / file /tmp/secret.txt`, ffmpeg with
//!     no `-f` picks the CONCAT demuxer and tries to read that path; only the
//!     `-safe` default stopped it ("Unsafe file name"). One `-safe 0` anywhere in
//!     the argument list, or a demuxer whose safety check is weaker, and that is
//!     local file disclosure. So the container is never inferred: `probeAv` reads
//!     it, an allowlist approves it, and it is FORCED with `-f`. Measured: the
//!     same evil file under `-f mov` dies at "moov atom not found".
//!     Note also that `-f lavfi` needs no protocol at all - the whitelist would
//!     not have saved us there either. Forcing the format is the load-bearing
//!     guard; the whitelist is depth behind it.
//!
//!  3. MOV DATA REFERENCES. An .mp4 can name its media data in another file.
//!     `use_absolute_path` defaults to false (confirmed in `-h demuxer=mov`:
//!     "this is a possible security issue (default false)"), so a dref can only
//!     reach paths relative to the input - which is the uploader's own storage
//!     directory. Passed explicitly anyway, because a default is a decision
//!     someone else is free to revisit.
//!
//!  4. LIFETIME. ffmpeg spawns worker threads; killing the parent leaves them
//!     running. Every process starts its own process GROUP and the timeout kills
//!     the group. A runaway encoder on a two-core container is not a leak, it is
//!     an outage.
//!
//! Rust specifics: a caller that drops the future of [`run_av_tool`] has gone
//! away, so the process group is killed then too (upstream's promise could not
//! be dropped; its callers passed the request's signal instead).

use std::ffi::{OsStr, OsString};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::OnceCell;

use crate::media::abort::AbortSignal;
use crate::runtime::limits::effective_cpu_count;

/// Overridable so a dev box can point at a local build; the image ships both.
static FFMPEG_BIN: LazyLock<OsString> = LazyLock::new(|| non_empty_env("FFMPEG_PATH", "ffmpeg"));
static FFPROBE_BIN: LazyLock<OsString> = LazyLock::new(|| non_empty_env("FFPROBE_PATH", "ffprobe"));

/// `process.env.X || fallback`: an empty value falls back too.
fn non_empty_env(key: &str, fallback: &str) -> OsString {
    match std::env::var_os(key) {
        Some(v) if !v.is_empty() => v,
        _ => OsString::from(fallback),
    }
}

/// Only ever these two. Nothing here takes a program name from a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvTool {
    Ffmpeg,
    Ffprobe,
}

impl AvTool {
    pub fn as_str(self) -> &'static str {
        match self {
            AvTool::Ffmpeg => "ffmpeg",
            AvTool::Ffprobe => "ffprobe",
        }
    }
}

impl std::fmt::Display for AvTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn binary_for(tool: AvTool) -> &'static OsStr {
    match tool {
        AvTool::Ffmpeg => &FFMPEG_BIN,
        AvTool::Ffprobe => &FFPROBE_BIN,
    }
}

/// Head of stderr kept for diagnosis. ffmpeg is chatty and the interesting line
/// is always near the top; keeping the whole stream would let a crafted file
/// write unbounded data into our memory through its error messages.
pub const STDERR_LIMIT: usize = 8 * 1024;

/// Ceiling on anything we collect from stdout (ffprobe JSON).
pub const STDOUT_LIMIT: usize = 512 * 1024;

/// `AvToolError`: the tool ran and exited non-zero (or died of a signal
/// nobody sent on purpose).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{tool} exited with {}: {}", code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into()), first_chars(stderr, 400))]
pub struct AvToolError {
    pub tool: AvTool,
    pub code: Option<i32>,
    pub stderr: String,
}

fn first_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// The job ran past its deadline and its process group was killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("ffmpeg exceeded {ms} ms and was killed")]
pub struct AvTimeoutError {
    pub ms: u64,
}

/// The caller went away; the process group was killed for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("ffmpeg abandoned by the caller")]
pub struct AvAbortedError;

#[derive(Debug, Clone, Default)]
pub struct RunAvOptions {
    /// Hard wall clock. On expiry the whole process GROUP is killed.
    pub timeout_ms: u64,
    /// Caller's signal; abort kills the group the same way a timeout does.
    pub signal: Option<AbortSignal>,
    /// Collect stdout (ffprobe JSON). Off for ffmpeg, which writes to a file.
    pub capture_stdout: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvResult {
    pub stdout: String,
    pub stderr: String,
}

#[cfg(unix)]
mod sys {
    use std::ffi::c_int;

    unsafe extern "C" {
        fn kill(pid: c_int, sig: c_int) -> c_int;
    }

    const SIGKILL: c_int = 9;

    /// `kill(-pid, SIGKILL)`; the leader alone when the group is gone and the
    /// leader has not been reaped yet (so its pid cannot have been recycled).
    pub fn kill_group(pid: u32, leader_reaped: bool) {
        let Ok(pid) = c_int::try_from(pid) else { return };
        // SAFETY: plain syscalls. The group id stays reserved while any member
        // lives, and the leader's pid while it is unreaped.
        unsafe {
            if kill(-pid, SIGKILL) != 0 && !leader_reaped {
                kill(pid, SIGKILL);
            }
        }
    }
}

/// Kill a whole process tree.
///
/// On POSIX the child is spawned as a process group leader, so the negative pid
/// reaches its children too. SIGKILL rather than SIGTERM: this path is only
/// reached once the deadline has already passed (or the caller is gone), and an
/// encoder that has ignored its deadline has forfeited the chance to shut down
/// tidily.
fn kill_tree(pid: Option<u32>, leader_reaped: bool, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        sys::kill_group(pid, leader_reaped);
        return;
    }
    if !leader_reaped {
        let _ = child.start_kill();
    }
}

/// Kills the group if the future running it is dropped before the process
/// has been reaped.
struct KillOnDrop {
    pid: Option<u32>,
    armed: bool,
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.armed
            && let Some(pid) = self.pid
        {
            sys::kill_group(pid, false);
        }
    }
}

/// Read a pipe to its end, keeping at most `limit` bytes.
///
/// The obvious form of this - `if (buf.length < LIMIT) buf += chunk` - is not a
/// bound at all: it checks BEFORE appending, so one chunk arriving under the
/// limit can carry the total arbitrarily past it. A pipe hands over whatever has
/// accumulated, so 64 KB of ffmpeg warnings arrived as a single chunk and the
/// 8 KB cap admitted all of it. Caught by a test that had to run on a loaded
/// machine to see the coalescing.
async fn read_bounded(pipe: Option<impl AsyncRead + Unpin>, limit: usize) -> String {
    let Some(mut pipe) = pipe else { return String::new() };
    let mut kept = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// Run one of the two tools with an argument VECTOR.
///
/// No shell is involved, so no argument is ever parsed by one: a filename
/// containing `;` or `$(…)` is a filename. Callers build their arguments from
/// fixed strings plus values that have been through the allowlists in
/// av_targets.rs, and paths that come from our own storage driver.
pub async fn run_av_tool<S: AsRef<OsStr>>(tool: AvTool, args: &[S], opts: RunAvOptions) -> anyhow::Result<AvResult> {
    run_av_program(binary_for(tool), tool, args, opts).await
}

/// [`run_av_tool`] with an explicit program (the tests point it at a stand-in).
pub(crate) async fn run_av_program<S: AsRef<OsStr>>(
    program: &OsStr,
    tool: AvTool,
    args: &[S],
    opts: RunAvOptions,
) -> anyhow::Result<AvResult> {
    // Refused before a process exists: upstream spawned and killed one, with
    // the same answer.
    if opts.signal.as_ref().is_some_and(AbortSignal::aborted) {
        return Err(AvAbortedError.into());
    }

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        // Inherit nothing that could redirect the tool. ffmpeg reads a handful
        // of variables (FFREPORT writes a log file wherever it is pointed), and
        // this process's environment holds database and mail credentials that a
        // media converter has no reason to see.
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    // Own process group, so the timeout can take the children with it.
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn()?;
    let pid = child.id();
    let mut guard = KillOnDrop { pid, armed: true };

    let stdout_limit = if opts.capture_stdout { STDOUT_LIMIT } else { 0 };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let readers = async { tokio::join!(read_bounded(stdout, stdout_limit), read_bounded(stderr, STDERR_LIMIT)) };
    tokio::pin!(readers);

    let deadline = tokio::time::sleep(Duration::from_millis(opts.timeout_ms));
    tokio::pin!(deadline);
    let abort = async {
        match &opts.signal {
            Some(s) => s.cancelled().await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(abort);

    // A deadline or an abort is the REAL reason, even though what we observe
    // afterwards is a process that died of SIGKILL. Reporting the signal would
    // turn "took too long" into "conversion failed".
    let mut outcome: Option<anyhow::Error> = None;
    let mut output: Option<(String, String)> = None;
    let mut status: Option<std::process::ExitStatus> = None;

    while status.is_none() || output.is_none() {
        tokio::select! {
            s = child.wait(), if status.is_none() => {
                status = Some(s?);
                guard.armed = false;
            }
            o = &mut readers, if output.is_none() => {
                output = Some(o);
            }
            _ = &mut deadline, if outcome.is_none() => {
                outcome = Some(AvTimeoutError { ms: opts.timeout_ms }.into());
                kill_tree(pid, status.is_some(), &mut child);
            }
            _ = &mut abort, if outcome.is_none() => {
                outcome = Some(AvAbortedError.into());
                kill_tree(pid, status.is_some(), &mut child);
            }
        }
    }

    if let Some(err) = outcome {
        return Err(err);
    }
    let (stdout, stderr) = output.unwrap_or_default();
    let status = status.expect("loop ends with a status");
    if !status.success() {
        return Err(AvToolError { tool, code: status.code(), stderr }.into());
    }
    Ok(AvResult { stdout, stderr })
}

/// Arguments every ffmpeg/ffprobe invocation starts with.
///
/// `container` is the demuxer name, and it must come from the allowlist in
/// av_probe.rs - see point 2 in the header. Passing `None` is only correct for
/// the probe that DISCOVERS the container, which is why that call is the one
/// place in the codebase allowed to omit it.
///
/// `before_input`: options that must sit BEFORE `-i` to apply to the input -
/// `-ss` above all, where the placement is the difference between seeking to a
/// keyframe and decoding the whole file up to that point.
pub fn input_args(path: &str, container: Option<&str>, before_input: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec!["-nostdin".into(), "-protocol_whitelist".into(), "file,pipe".into()];
    if let Some(c) = container.filter(|c| !c.is_empty()) {
        args.push("-f".into());
        args.push(c.into());
    }
    // Demuxer-PRIVATE option: ffmpeg resolves it against the chosen demuxer's
    // option table and exits with "Option not found" if that demuxer has no such
    // knob. So it may only be passed for the family that owns it, which is also
    // the only family that has data references to confine.
    if container == Some("mov") {
        args.push("-use_absolute_path".into());
        args.push("0".into());
    }
    args.extend(before_input.iter().cloned());
    args.push("-i".into());
    args.push(path.into());
    args
}

/// Probe arguments: same guards, but ffprobe takes its input positionally.
pub fn probe_input_args(path: &str) -> Vec<String> {
    vec!["-protocol_whitelist".into(), "file,pipe".into(), "-i".into(), path.into()]
}

/// Threads one ffmpeg job may use.
///
/// The cgroup number, not `nproc` - inside the container those disagree 2 vs 6
/// (runtime/limits.rs). Handing an encoder six threads on a two-core quota
/// does not make it faster, it makes the scheduler throttle the whole cgroup and
/// every other request on the box slower with it.
pub fn thread_args() -> Vec<String> {
    vec!["-threads".into(), effective_cpu_count().to_string()]
}

/// Is ffmpeg actually present? Cached: the answer cannot change under us.
static AVAILABLE: OnceCell<bool> = OnceCell::const_new();

pub async fn ffmpeg_available() -> bool {
    *AVAILABLE
        .get_or_init(|| async {
            run_av_tool(
                AvTool::Ffprobe,
                &["-version"],
                RunAvOptions { timeout_ms: 5_000, signal: None, capture_stdout: true },
            )
            .await
            .is_ok()
        })
        .await
}

#[cfg(test)]
mod tests {
    //! tests/integration/ffmpeg-runner.test.ts
    //!
    //! The spawn wrapper, tested against `/bin/sh` standing in for ffmpeg
    //! (upstream pointed FFPROBE_PATH at the Node binary). The things worth
    //! testing here are not about video at all: does a runaway process really
    //! die, does an abandoned one really die, does a non-zero exit surface as an
    //! error rather than as empty output, and is the environment really
    //! withheld. Those are the properties that keep a public conversion endpoint
    //! from becoming a way to pin both cores.
    use super::*;
    use crate::media::abort::AbortController;

    fn sh() -> &'static OsStr {
        OsStr::new("/bin/sh")
    }

    /// Run a snippet of shell through the stand-in "tool".
    async fn script(src: &str, opts: RunAvOptions) -> anyhow::Result<AvResult> {
        run_av_program(sh(), AvTool::Ffprobe, &["-c", src], opts).await
    }

    fn opts(timeout_ms: u64, capture_stdout: bool) -> RunAvOptions {
        RunAvOptions { timeout_ms, signal: None, capture_stdout }
    }

    /// Is a pid running, gone, or a zombie? A zombie still answers signal 0,
    /// so only the process table tells them apart.
    fn process_state(pid: u32) -> &'static str {
        let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output();
        match out {
            Ok(o) => {
                let stat = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if stat.is_empty() {
                    "gone"
                } else if stat.starts_with('Z') {
                    "zombie"
                } else {
                    "running"
                }
            }
            Err(_) => "gone",
        }
    }

    #[tokio::test]
    async fn returns_stdout_when_the_tool_succeeds() {
        let res = script(r#"printf '{"ok":1}'"#, opts(10_000, true)).await.unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&res.stdout).unwrap(), serde_json::json!({"ok": 1}));
    }

    #[tokio::test]
    async fn reports_a_non_zero_exit_as_an_error_carrying_the_stderr() {
        // Silently treating a failed conversion as an empty file is how a zero-byte
        // download reaches a user with a 200 on it.
        let err = script("echo 'moov atom not found' >&2; exit 1", opts(10_000, false)).await.unwrap_err();
        let tool_err = err.downcast_ref::<AvToolError>().expect("AvToolError");
        assert_eq!(tool_err.code, Some(1));
        assert!(tool_err.stderr.contains("moov atom not found"));

        let err = script("echo boom >&2; exit 3", opts(10_000, false)).await.unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        assert!(err.to_string().starts_with("ffprobe exited with 3: "), "{err}");
    }

    #[tokio::test]
    async fn kills_a_process_that_ignores_its_deadline() {
        // A tight loop that never finishes on its own, so the error type is the
        // whole proof. Deliberately no wall-clock assertion (a loaded machine).
        let err = script("while :; do :; done", opts(700, false)).await.unwrap_err();
        assert_eq!(err.downcast_ref::<AvTimeoutError>(), Some(&AvTimeoutError { ms: 700 }));
    }

    #[tokio::test]
    async fn kills_the_whole_process_group_not_just_the_parent() {
        // ffmpeg's own workers are threads and die with the process, but the
        // guarantee has to be about the tree, because a survivor on a two-core
        // container is an outage rather than a leak. The grandchild's pid goes to
        // a FILE: the call fails with a timeout, and a failure carries no output.
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let src = format!("sleep 1000 & echo $! > '{}'; while :; do :; done", pid_file.display());
        let err = script(&src, opts(1500, false)).await.unwrap_err();
        assert!(err.is::<AvTimeoutError>());

        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        // Poll rather than sleep a fixed amount: delivering SIGKILL and reaping is
        // fast but not instant.
        for _ in 0..40 {
            if process_state(pid) != "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_ne!(process_state(pid), "running", "the grandchild outlived the process group kill");
    }

    #[tokio::test]
    async fn drops_a_job_whose_caller_has_gone_away() {
        // The abort waits for the process to say it is running, rather than firing
        // on a timer that races the spawn.
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let controller = AbortController::new();
        let src = format!("echo 1 > '{}'; while :; do :; done", ready.display());
        let signal = controller.signal();
        let job = tokio::spawn(async move {
            script(&src, RunAvOptions { timeout_ms: 30_000, signal: Some(signal), capture_stdout: false }).await
        });
        for _ in 0..100 {
            if ready.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(ready.exists(), "the stand-in tool never started");
        controller.abort();
        assert!(job.await.unwrap().unwrap_err().is::<AvAbortedError>());
    }

    #[tokio::test]
    async fn a_dropped_caller_kills_the_group_too() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let src = format!("echo $$ > '{}'; while :; do :; done", pid_file.display());
        let job = tokio::spawn(async move { script(&src, opts(30_000, false)).await });
        for _ in 0..100 {
            if pid_file.exists() && !std::fs::read_to_string(&pid_file).unwrap().trim().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        job.abort();
        let _ = job.await;
        for _ in 0..40 {
            if process_state(pid) != "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_ne!(process_state(pid), "running");
    }

    #[tokio::test]
    async fn refuses_immediately_when_the_caller_has_already_gone() {
        let controller = AbortController::new();
        controller.abort();
        let err = script(
            "exit 0",
            RunAvOptions { timeout_ms: 5_000, signal: Some(controller.signal()), capture_stdout: false },
        )
        .await
        .unwrap_err();
        assert!(err.is::<AvAbortedError>());
    }

    #[tokio::test]
    async fn hands_the_tool_nothing_but_path() {
        // This process holds database and mail credentials in its environment. A
        // media converter has no reason to see them, and ffmpeg reads variables of
        // its own (FFREPORT writes a log wherever it is pointed). Cargo's own
        // variables stand in for the secrets here: they are certainly set.
        assert!(std::env::var_os("CARGO_PKG_NAME").is_some());
        let res = script("env", opts(10_000, true)).await.unwrap();
        let keys: Vec<&str> = res.stdout.lines().filter_map(|l| l.split('=').next()).collect();
        assert!(keys.contains(&"PATH"));
        for key in &keys {
            // What sh itself exports into `env`, and nothing else.
            assert!(["PATH", "PWD", "SHLVL", "_", "OLDPWD"].contains(key), "leaked {key}");
        }
    }

    #[tokio::test]
    async fn treats_a_missing_binary_as_a_rejection_not_a_crash() {
        let err =
            run_av_program(OsStr::new("/nonexistent/ffprobe"), AvTool::Ffprobe, &["-version"], opts(5_000, false))
                .await
                .unwrap_err();
        assert!(err.downcast_ref::<std::io::Error>().is_some(), "{err}");
    }

    #[tokio::test]
    async fn bounds_how_much_a_tool_can_write_into_our_memory() {
        // A crafted file can make ffmpeg emit warnings without end; keeping all of
        // them would turn its error messages into the memory leak. Half a megabyte,
        // so the writes cannot all arrive as one chunk.
        let res = script("yes xxxxxxxxxxxxxxx | head -c 600000 >&2; exit 0", opts(15_000, false)).await.unwrap();
        assert!(res.stderr.len() <= STDERR_LIMIT);
        // ...and it really did capture something, or the bound proves nothing.
        assert!(!res.stderr.is_empty());
    }

    #[tokio::test]
    async fn bounds_captured_stdout_the_same_way() {
        let res = script("yes yyyyyyyyyyyyyyy | head -c 1048576", opts(15_000, true)).await.unwrap();
        assert!(res.stdout.len() <= STDOUT_LIMIT);
        assert!(!res.stdout.is_empty());
    }

    #[tokio::test]
    async fn ignores_stdout_unless_asked_but_still_drains_it() {
        let res = script("yes zzzzzzzzzzzzzzz | head -c 1048576", opts(15_000, false)).await.unwrap();
        assert!(res.stdout.is_empty());
    }

    #[test]
    fn builds_the_input_guards() {
        assert_eq!(
            input_args("/s/a.mp4", Some("mov"), &["-ss".into(), "3".into()]),
            [
                "-nostdin",
                "-protocol_whitelist",
                "file,pipe",
                "-f",
                "mov",
                "-use_absolute_path",
                "0",
                "-ss",
                "3",
                "-i",
                "/s/a.mp4"
            ]
        );
        assert_eq!(
            input_args("/s/a.mkv", Some("matroska"), &[]),
            ["-nostdin", "-protocol_whitelist", "file,pipe", "-f", "matroska", "-i", "/s/a.mkv"]
        );
        assert_eq!(input_args("/s/a", None, &[]), ["-nostdin", "-protocol_whitelist", "file,pipe", "-i", "/s/a"]);
        assert_eq!(probe_input_args("/s/a"), ["-protocol_whitelist", "file,pipe", "-i", "/s/a"]);
        assert_eq!(thread_args()[0], "-threads");
    }

    #[tokio::test]
    async fn finds_the_real_ffprobe() {
        // The image ships ffmpeg; so does this Mac. Skip quietly elsewhere.
        if std::process::Command::new(binary_for(AvTool::Ffprobe)).arg("-version").output().is_ok() {
            assert!(ffmpeg_available().await);
        }
    }
}
