//! Background jobs: the timers `server.ts` starts after boot.
//!
//! | Job | First run | Then | Upstream |
//! |---|---|---|---|
//! | tus sweep | immediately at boot | every hour from boot | `sweepTusUploads` (lib/upload/tus.ts) |
//! | trash purge | 60 s after boot | every 6 hours from boot | `purgeExpiredTrash` (lib/maintenance/trash.ts) |
//! | mail settings warm-up | once, before listening (not awaited) | - | `warmMailSettings` (lib/mail-settings.ts) |
//!
//! Like `setInterval(...).unref()` upstream, the timers never keep the process
//! alive: they are aborted on shutdown. A failing run is logged and the timer
//! keeps going. One difference by design: a run that takes longer than its
//! interval delays the next one instead of overlapping it.
//!
//! Adding a job: write the work as a function in the module that owns it
//! (upstream path), then add one entry to [`jobs`].

use std::time::Duration;

use futures_util::future::BoxFuture;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior};

use crate::db::types::JsDate;
use crate::state::AppState;

/// One periodic job.
pub struct Job {
    pub name: &'static str,
    /// Delay before the first run, or `None` to wait for the first interval.
    pub first_run: Option<Duration>,
    /// Interval, counted from boot like `setInterval`.
    pub every: Duration,
    pub run: fn(AppState) -> BoxFuture<'static, ()>,
}

fn tus_sweep(state: AppState) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        match crate::upload::tus::sweep_tus_uploads(&state).await {
            Ok(n) if n > 0 => tracing::info!("[tus] swept {n} stale upload record(s)"),
            Ok(_) => {}
            Err(err) => tracing::error!("[tus] cleanup failed: {err:#}"),
        }
    })
}

fn trash_purge(state: AppState) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        match crate::maintenance::trash::purge_expired_trash(&state, JsDate::now()).await {
            Ok(r) if r.deleted > 0 || r.skipped > 0 => tracing::info!(
                "[trash] purged {} expired item(s), {} byte(s) reclaimed, {} retried next sweep",
                r.deleted,
                r.bytes,
                r.skipped
            ),
            Ok(_) => {}
            Err(err) => tracing::error!("[trash] purge failed: {err:#}"),
        }
    })
}

/// Every periodic job. ONE ENTRY PER JOB.
pub fn jobs() -> Vec<Job> {
    vec![
        Job {
            name: "tus-sweep",
            // Clear whatever a previous run left behind.
            first_run: Some(Duration::ZERO),
            every: crate::upload::tus::TUS_CLEANUP_INTERVAL,
            run: tus_sweep,
        },
        Job {
            name: "trash-purge",
            // Delayed so a boot loop can never turn into a delete loop, and so
            // startup is not competing with it for the database.
            first_run: Some(crate::maintenance::trash::TRASH_FIRST_SWEEP_DELAY),
            every: crate::maintenance::trash::TRASH_SWEEP_INTERVAL,
            run: trash_purge,
        },
    ]
}

/// Start the one-off boot tasks and every periodic job. Dropping (or
/// `abort_all` on) the returned set stops them.
pub fn start(state: &AppState) -> JoinSet<()> {
    let mut set = JoinSet::new();
    // `void warmMailSettings()`: before traffic arrives, not awaited.
    let warm = state.clone();
    set.spawn(async move { crate::mail_settings::warm_mail_settings(&warm).await });
    for job in jobs() {
        set.spawn(run_job(state.clone(), job));
    }
    set
}

async fn run_job(state: AppState, job: Job) {
    let boot = Instant::now();
    if let Some(delay) = job.first_run {
        let state = state.clone();
        let run = job.run;
        // The first run is a timer of its own, as upstream's setTimeout was.
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            run(state).await;
        });
    }
    let mut ticker = tokio::time::interval_at(boot + job.every, job.every);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        tracing::debug!("job {} running", job.name);
        (job.run)(state.clone()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_matches_server_ts() {
        let jobs = jobs();
        let tus = jobs.iter().find(|j| j.name == "tus-sweep").unwrap();
        assert_eq!(tus.first_run, Some(Duration::ZERO));
        assert_eq!(tus.every, Duration::from_secs(3600));
        let trash = jobs.iter().find(|j| j.name == "trash-purge").unwrap();
        assert_eq!(trash.first_run, Some(Duration::from_secs(60)));
        assert_eq!(trash.every, Duration::from_secs(6 * 3600));
    }
}
