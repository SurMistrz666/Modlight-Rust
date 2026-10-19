//! Port of `lib/media/transform-queue.ts`: the one gate every expensive media
//! operation goes through.
//!
//! Thumbnails, format conversion and (next) video transcoding all decode
//! attacker-supplied files with native code on a container that is allowed two
//! cores and 2 GiB. `/convert` is reachable by anyone holding a public link, so
//! "how many of these may run at once" cannot be answered per feature - it has to
//! be answered once, for all of them, or the next feature quietly doubles the
//! ceiling the previous one was sized against.
//!
//! Three properties, and each one exists because its absence was a real hole:
//!
//!  1. WEIGHTED capacity. One limiter, as it must be, but a 39 ms thumbnail must
//!     not queue behind two 5 s AVIF encodes for a slot it barely uses. The
//!     weights and the pool they divide are in `media::transform_cost`.
//!
//!  2. CANCELLATION. A caller that has gone away is not worth working for. Eight
//!     requests fired at /convert (two slots plus six queue) and then abandoned
//!     used to be eight jobs that ran to completion against nobody, which made
//!     holding the box at full load free. Aborting while queued is exact; a job
//!     already inside libvips can only be asked (see the deadline note below).
//!
//!  3. A DEADLINE on the response. The slot is released when the work really
//!     ends, never when the deadline fires - releasing early would let a fresh
//!     caller start while the abandoned job still holds its memory, which is the
//!     one thing the limiter exists to prevent.
//!
//! Deliberately in-process and dependency-free: there is exactly one server
//! process behind the proxy, so a counter is the whole mechanism.
//!
//! Rust specifics, same behaviour: the job is spawned as its own task so that
//! its slot follows the job and not the caller; a caller that drops the
//! `run` future has "gone away", which aborts the job's signal exactly like
//! the request's `signal` firing upstream (and, while it is still queued,
//! removes it from the queue).

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::media::abort::{AbortController, AbortReason, AbortSignal};
use crate::media::transform_cost::{TRANSFORM_CAPACITY, TRANSFORM_QUEUE};

/// No slot, and no room to wait for one. The caller should retry later (429/503).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Too many transforms in progress.")]
pub struct BusyError;

/// The caller went away (socket closed) before the work could start or finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Transform abandoned by the caller.")]
pub struct AbortedError;

/// The work outlived its deadline. The response gives up; the job may not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Transform exceeded {ms} ms.")]
pub struct TimeoutError {
    pub ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct LimiterOptions {
    /// Total capacity, in weight units.
    pub limit: u32,
    /// How many callers may wait for capacity before further ones are refused.
    pub queue: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Units of capacity this job occupies (default 1). Clamped to `limit`.
    pub weight: Option<u32>,
    /// Wall clock after which the CALLER stops waiting.
    pub timeout_ms: Option<u64>,
    /// The request's signal, so an abandoned job is dropped rather than performed.
    pub signal: Option<AbortSignal>,
}

struct Waiter {
    id: u64,
    weight: u32,
    admit: oneshot::Sender<()>,
}

#[derive(Default)]
struct State {
    used: u32,
    running: usize,
    waiting: VecDeque<Waiter>,
    next_id: u64,
}

struct Shared {
    limit: u32,
    queue: usize,
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Admit whoever is at the FRONT and fits, and only from the front.
    ///
    /// Skipping a waiter that does not fit in favour of a lighter one behind it
    /// would starve every heavy job the moment thumbnails are flowing: a conversion
    /// would never see two free units again.
    fn drain(&self, st: &mut State) {
        while let Some(front) = st.waiting.front() {
            if st.used + front.weight > self.limit {
                break;
            }
            let next = st.waiting.pop_front().expect("front exists");
            st.used += next.weight;
            // A waiter whose future is already gone settles this weight in its
            // own guard (it finds itself no longer queued), so a failed send
            // changes nothing here.
            let _ = next.admit.send(());
        }
    }

    fn release(&self, weight: u32) {
        let mut st = self.lock();
        st.used -= weight;
        self.drain(&mut st);
    }
}

/// `Limiter`: `run` plus the two diagnostics.
#[derive(Clone)]
pub struct Limiter {
    shared: Arc<Shared>,
}

/// `createLimiter({ limit, queue })`.
pub fn create_limiter(opts: LimiterOptions) -> Limiter {
    Limiter { shared: Arc::new(Shared { limit: opts.limit, queue: opts.queue, state: Mutex::new(State::default()) }) }
}

fn as_error(reason: Option<AbortReason>) -> anyhow::Error {
    match reason {
        Some(AbortReason::TimedOut { ms }) => TimeoutError { ms }.into(),
        _ => AbortedError.into(),
    }
}

/// Settles a queued waiter whose future is dropped before it was handled:
/// removed from the queue if still there, its capacity given back if `drain`
/// had already admitted it.
struct WaiterGuard<'a> {
    shared: &'a Shared,
    id: u64,
    weight: u32,
    settled: bool,
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut st = self.shared.lock();
        if let Some(pos) = st.waiting.iter().position(|w| w.id == self.id) {
            st.waiting.remove(pos);
        } else {
            st.used -= self.weight;
            self.shared.drain(&mut st);
        }
    }
}

/// Held by the spawned job: gives the slot back when the work really ends,
/// however it ends (value, error or panic).
struct SlotGuard {
    shared: Arc<Shared>,
    weight: u32,
    detach: CancellationToken,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        {
            let mut st = self.shared.lock();
            st.running -= 1;
        }
        self.detach.cancel();
        self.shared.release(self.weight);
    }
}

impl Limiter {
    /// Running + waiting jobs, for tests and diagnostics.
    pub fn depth(&self) -> usize {
        let st = self.shared.lock();
        st.running + st.waiting.len()
    }

    /// Capacity units currently held, for tests and diagnostics.
    pub fn used(&self) -> u32 {
        self.shared.lock().used
    }

    /// `giveUp` is the COMBINED signal: the caller hanging up and the deadline
    /// expiring both land on it. Queueing has to be covered by the deadline too -
    /// a caller stuck behind a job libvips will not interrupt would otherwise wait
    /// for as long as that job takes, with its own timeout not yet started.
    async fn acquire(&self, weight: u32, give_up: &AbortSignal) -> anyhow::Result<()> {
        let shared = &*self.shared;
        let (id, rx) = {
            let mut st = shared.lock();
            // New callers never overtake the queue: admitting one because it happens to
            // fit would put more work in flight than the limit allows and push the
            // waiters further back every time.
            if st.waiting.is_empty() && st.used + weight <= shared.limit {
                st.used += weight;
                return Ok(());
            }
            if st.waiting.len() >= shared.queue {
                return Err(BusyError.into());
            }
            let id = st.next_id;
            st.next_id += 1;
            let (tx, rx) = oneshot::channel();
            st.waiting.push_back(Waiter { id, weight, admit: tx });
            (id, rx)
        };
        let mut guard = WaiterGuard { shared, id, weight, settled: false };

        tokio::select! {
            admitted = rx => {
                guard.settled = true;
                if admitted.is_err() {
                    // The limiter dropped the waiter without admitting it; cannot
                    // happen while the limiter is alive.
                    return Err(AbortedError.into());
                }
                // Admitted - but the caller may have hung up in the very same tick.
                if give_up.aborted() {
                    shared.release(weight);
                    return Err(as_error(give_up.reason()));
                }
                Ok(())
            }
            _ = give_up.cancelled() => {
                guard.settled = true;
                let mut st = shared.lock();
                if let Some(pos) = st.waiting.iter().position(|w| w.id == id) {
                    st.waiting.remove(pos);
                } else {
                    // Already admitted; the slot is ours to hand back.
                    st.used -= weight;
                    shared.drain(&mut st);
                }
                Err(as_error(give_up.reason()))
            }
        }
    }

    /// `run(fn, { weight, timeoutMs, signal })`.
    ///
    /// `f` receives the combined signal (caller gone, or deadline passed) and
    /// may bail out at a checkpoint; a job that cannot be interrupted still
    /// finishes, holding its slot, with nobody waiting for it. Errors are the
    /// job's own, or [`BusyError`], [`AbortedError`], [`TimeoutError`]
    /// (`err.downcast_ref::<BusyError>()` is upstream's `instanceof`).
    pub async fn run<T, F, Fut>(&self, f: F, opts: RunOptions) -> anyhow::Result<T>
    where
        F: FnOnce(AbortSignal) -> Fut,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let weight = opts.weight.unwrap_or(1).max(1).min(self.shared.limit);
        let outer = opts.signal;
        if outer.as_ref().is_some_and(AbortSignal::aborted) {
            return Err(AbortedError.into());
        }

        // One controller for the whole call: it covers the wait for a slot AND the
        // work itself, and it carries WHY (TimeoutError vs AbortedError) so the
        // route can answer 503 or 499 without guessing.
        let deadline = AbortController::new();
        let detach = CancellationToken::new();
        if opts.timeout_ms.is_some() || outer.is_some() {
            let (deadline, detach, outer, timeout_ms) =
                (deadline.clone(), detach.clone(), outer.clone(), opts.timeout_ms);
            // A timer must never be the reason the process stays alive: this task
            // ends with the work (`detach`) or when it fires.
            tokio::spawn(async move {
                let timer = async {
                    match timeout_ms {
                        Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                        None => std::future::pending().await,
                    }
                };
                let relay = async {
                    match &outer {
                        Some(s) => s.cancelled().await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = detach.cancelled() => {}
                    _ = timer => deadline.abort_with(AbortReason::TimedOut { ms: timeout_ms.unwrap_or(0) }),
                    _ = relay => deadline.abort_with(AbortReason::Aborted),
                }
            });
        }
        // Until the job owns the slot, leaving this function (an error, or the
        // caller dropping this future while it waits) stops the watcher.
        let detach = detach.drop_guard();
        // Dropping this future means the caller went away.
        let caller_gone = deadline.abort_on_drop();
        let signal = deadline.signal();

        if let Err(err) = self.acquire(weight, &signal).await {
            caller_gone.disarm();
            return Err(err);
        }

        // From here the slot is held until `f`'s future really settles.
        self.shared.lock().running += 1;
        let slot = SlotGuard { shared: self.shared.clone(), weight, detach: detach.disarm() };
        let work = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(signal.clone()))) {
            Ok(work) => work,
            Err(panic) => {
                // A synchronous throw still has to give the slot back.
                drop(slot);
                caller_gone.disarm();
                std::panic::resume_unwind(panic);
            }
        };

        // Accounting follows the REAL end of the work, not what the caller is told.
        let mut handle = tokio::spawn(async move {
            let _slot = slot;
            work.await
        });

        let joined = if opts.timeout_ms.is_none() && outer.is_none() {
            Some((&mut handle).await)
        } else {
            // The caller waits only until the deadline. A job that cannot be
            // interrupted (libvips does not always come back to its progress callback)
            // still finishes - it just finishes with nobody on the other end.
            tokio::select! {
                biased;
                joined = &mut handle => Some(joined),
                _ = signal.cancelled() => None,
            }
        };
        caller_gone.disarm();
        match joined {
            Some(Ok(result)) => result,
            Some(Err(join)) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
            Some(Err(_)) => Err(AbortedError.into()),
            None => Err(as_error(signal.reason())),
        }
    }
}

/// The shared limiter. Capacity, weights and queue depth are derived in
/// `media::transform_cost` from the container's real memory limit.
pub static IMAGE_TRANSFORMS: LazyLock<Limiter> =
    LazyLock::new(|| create_limiter(LimiterOptions { limit: TRANSFORM_CAPACITY, queue: TRANSFORM_QUEUE }));

/// `imageTransforms`.
pub fn image_transforms() -> &'static Limiter {
    &IMAGE_TRANSFORMS
}

#[cfg(test)]
mod tests {
    //! tests/unit/transform-queue.test.ts
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use super::*;

    /// A future whose settling this test controls.
    struct Gate {
        tx: Option<oneshot::Sender<()>>,
        rx: Option<oneshot::Receiver<()>>,
    }

    impl Gate {
        fn new() -> Gate {
            let (tx, rx) = oneshot::channel();
            Gate { tx: Some(tx), rx: Some(rx) }
        }
        fn wait(&mut self) -> impl Future<Output = ()> + Send + 'static {
            let rx = self.rx.take().expect("one waiter");
            async move {
                let _ = rx.await;
            }
        }
        fn resolve(&mut self) {
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(());
            }
        }
    }

    async fn tick() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    fn limiter(limit: u32, queue: usize) -> Limiter {
        create_limiter(LimiterOptions { limit, queue })
    }

    fn spawn_run<T: Send + 'static>(
        l: &Limiter,
        work: impl Future<Output = anyhow::Result<T>> + Send + 'static,
        opts: RunOptions,
    ) -> tokio::task::JoinHandle<anyhow::Result<T>> {
        let l = l.clone();
        tokio::spawn(async move { l.run(move |_| work, opts).await })
    }

    // describe("the transform limiter")

    #[tokio::test]
    async fn runs_up_to_the_limit_at_once_and_makes_the_rest_wait() {
        let l = limiter(2, 4);
        let started = Arc::new(AtomicU32::new(0));
        let mut gates: Vec<Gate> = (0..3).map(|_| Gate::new()).collect();
        let jobs: Vec<_> = gates
            .iter_mut()
            .map(|gate| {
                let started = started.clone();
                let wait = gate.wait();
                spawn_run(
                    &l,
                    async move {
                        started.fetch_add(1, Ordering::SeqCst);
                        wait.await;
                        Ok(started.load(Ordering::SeqCst))
                    },
                    RunOptions::default(),
                )
            })
            .collect();

        // Two slots, so the third job has not been entered at all.
        tick().await;
        assert_eq!(started.load(Ordering::SeqCst), 2);

        gates[0].resolve();
        tick().await;
        assert_eq!(started.load(Ordering::SeqCst), 3);

        gates[1].resolve();
        gates[2].resolve();
        for j in jobs {
            j.await.unwrap().unwrap();
        }
        tick().await;
        assert_eq!(l.depth(), 0);
    }

    #[tokio::test]
    async fn refuses_rather_than_queueing_without_bound() {
        // /convert is reachable by anyone holding a public link and costs seconds of
        // CPU, so an unbounded queue is just a slower way to fall over.
        let l = limiter(1, 1);
        let mut gate = Gate::new();
        let wait = gate.wait();
        let running = spawn_run(
            &l,
            async move {
                wait.await;
                Ok(())
            },
            RunOptions::default(),
        );
        tick().await;
        let queued = spawn_run(&l, async { Ok("queued") }, RunOptions::default());
        tick().await;

        let third = l.run(|_| async { Ok("third") }, RunOptions::default()).await;
        assert!(third.unwrap_err().is::<BusyError>());

        gate.resolve();
        running.await.unwrap().unwrap();
        assert_eq!(queued.await.unwrap().unwrap(), "queued");
    }

    #[tokio::test]
    async fn hands_the_slot_to_the_next_waiter_even_when_a_job_throws() {
        let l = limiter(1, 2);
        let mut gate = Gate::new();
        let wait = gate.wait();
        let failing = spawn_run::<()>(
            &l,
            async move {
                wait.await;
                Err(anyhow::anyhow!("encoder blew up"))
            },
            RunOptions::default(),
        );
        tick().await;
        let waiting = spawn_run(&l, async { Ok("ran anyway") }, RunOptions::default());
        tick().await;

        gate.resolve();
        assert_eq!(failing.await.unwrap().unwrap_err().to_string(), "encoder blew up");
        assert_eq!(waiting.await.unwrap().unwrap(), "ran anyway");
        tick().await;
        assert_eq!(l.depth(), 0);
    }

    #[tokio::test]
    async fn never_lets_a_fresh_caller_overtake_the_queue_and_oversubscribe() {
        let l = limiter(1, 4);
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let body = |concurrent: Arc<AtomicUsize>, peak: Arc<AtomicUsize>| async move {
            let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            tokio::task::yield_now().await;
            concurrent.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        };

        let mut gate = Gate::new();
        let wait = gate.wait();
        let first = spawn_run(
            &l,
            {
                let (concurrent, peak) = (concurrent.clone(), peak.clone());
                async move {
                    let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    wait.await;
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            RunOptions::default(),
        );
        tick().await;
        let second = spawn_run(&l, body(concurrent.clone(), peak.clone()), RunOptions::default());
        tick().await;
        gate.resolve();
        let third = spawn_run(&l, body(concurrent.clone(), peak.clone()), RunOptions::default());

        for j in [first, second, third] {
            j.await.unwrap().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    // describe("weights")

    #[tokio::test]
    async fn never_lets_the_running_jobs_exceed_the_capacity() {
        // Four units: two conversions (2 each), or four thumbnails, never both.
        let l = limiter(4, 8);
        let held = Arc::new(AtomicU32::new(0));
        let peak = Arc::new(AtomicU32::new(0));
        let mut gates: Vec<Gate> = (0..6).map(|_| Gate::new()).collect();
        let mut jobs = Vec::new();
        for (i, gate) in gates.iter_mut().enumerate() {
            let weight = if i % 2 == 0 { 2 } else { 1 };
            let (held, peak, wait) = (held.clone(), peak.clone(), gate.wait());
            jobs.push(spawn_run(
                &l,
                async move {
                    let now = held.fetch_add(weight, Ordering::SeqCst) + weight;
                    peak.fetch_max(now, Ordering::SeqCst);
                    wait.await;
                    held.fetch_sub(weight, Ordering::SeqCst);
                    Ok(())
                },
                RunOptions { weight: Some(weight), ..Default::default() },
            ));
            tick().await;
        }

        tick().await;
        assert!(peak.load(Ordering::SeqCst) <= 4);
        for g in gates.iter_mut() {
            g.resolve();
        }
        for j in jobs {
            j.await.unwrap().unwrap();
        }
        tick().await;
        assert_eq!(l.used(), 0);
        assert!(peak.load(Ordering::SeqCst) <= 4);
    }

    #[tokio::test]
    async fn does_not_let_light_jobs_starve_a_heavy_one_waiting_in_front_of_them() {
        let l = limiter(2, 4);
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let mut running = Gate::new();
        let mut heavy_gate = Gate::new();

        let wait = running.wait();
        let first = spawn_run(
            &l,
            async move {
                wait.await;
                Ok(())
            },
            RunOptions { weight: Some(1), ..Default::default() },
        );
        tick().await;
        // Needs both units, so it must wait...
        let heavy = spawn_run(
            &l,
            {
                let (order, wait) = (order.clone(), heavy_gate.wait());
                async move {
                    order.lock().unwrap().push("heavy");
                    wait.await;
                    Ok(())
                }
            },
            RunOptions { weight: Some(2), ..Default::default() },
        );
        tick().await;
        // ...and this one fits in the free unit, but is behind it in the queue.
        let light = spawn_run(
            &l,
            {
                let order = order.clone();
                async move {
                    order.lock().unwrap().push("light");
                    Ok(())
                }
            },
            RunOptions::default(),
        );
        tick().await;

        running.resolve();
        first.await.unwrap().unwrap();
        tick().await;
        assert_eq!(*order.lock().unwrap(), vec!["heavy"]);

        heavy_gate.resolve();
        heavy.await.unwrap().unwrap();
        light.await.unwrap().unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["heavy", "light"]);
    }

    // describe("cancellation")

    #[tokio::test]
    async fn refuses_a_caller_that_has_already_gone_away() {
        let l = limiter(1, 1);
        let ac = AbortController::new();
        ac.abort();
        let ran = Arc::new(AtomicU32::new(0));
        let r = l
            .run(
                {
                    let ran = ran.clone();
                    move |_| async move {
                        ran.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                },
                RunOptions { signal: Some(ac.signal()), ..Default::default() },
            )
            .await;
        assert!(r.unwrap_err().is::<AbortedError>());
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        assert_eq!(l.depth(), 0);
    }

    #[tokio::test]
    async fn drops_a_queued_job_when_its_caller_hangs_up_and_frees_the_queue_slot() {
        // The scenario that made holding the box at full load free: fire two slots
        // plus six queued conversions, close every socket, and the server used to
        // work through all eight anyway.
        let l = limiter(1, 2);
        let mut gate = Gate::new();
        let wait = gate.wait();
        let running = spawn_run(
            &l,
            async move {
                wait.await;
                Ok(())
            },
            RunOptions::default(),
        );
        tick().await;

        let ac = AbortController::new();
        let queued_ran = Arc::new(AtomicU32::new(0));
        let queued = spawn_run(
            &l,
            {
                let queued_ran = queued_ran.clone();
                async move {
                    queued_ran.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            RunOptions { signal: Some(ac.signal()), ..Default::default() },
        );

        tick().await;
        assert_eq!(l.depth(), 2);

        ac.abort();
        assert!(queued.await.unwrap().unwrap_err().is::<AbortedError>());
        assert_eq!(l.depth(), 1); // the abandoned waiter is gone, not merely ignored

        gate.resolve();
        running.await.unwrap().unwrap();
        assert_eq!(queued_ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn passes_a_signal_into_the_job_so_a_running_one_can_bail_out_at_a_checkpoint() {
        let l = limiter(1, 1);
        let ac = AbortController::new();
        let mut started = Gate::new();
        let started_wait = started.wait();
        let (started_tx, ac_signal) = (Arc::new(Mutex::new(Some(started))), ac.signal());
        let job = {
            let l = l.clone();
            let started_tx = started_tx.clone();
            tokio::spawn(async move {
                l.run(
                    move |signal| async move {
                        if let Some(mut g) = started_tx.lock().unwrap().take() {
                            g.resolve();
                        }
                        tick().await;
                        if signal.aborted() {
                            return Err(AbortedError.into());
                        }
                        Ok("finished")
                    },
                    RunOptions { signal: Some(ac_signal), ..Default::default() },
                )
                .await
            })
        };

        started_wait.await;
        ac.abort();
        assert!(job.await.unwrap().unwrap_err().is::<AbortedError>());
        // The accounting follows the job's own future, one step behind the
        // answer the caller got. That ordering is deliberate - see the deadline test.
        tick().await;
        tick().await;
        assert_eq!(l.depth(), 0);
    }

    #[tokio::test]
    async fn a_dropped_caller_is_a_caller_that_hung_up() {
        let l = limiter(1, 2);
        let mut gate = Gate::new();
        let wait = gate.wait();
        let running = spawn_run(
            &l,
            async move {
                wait.await;
                Ok(())
            },
            RunOptions::default(),
        );
        tick().await;
        let queued = spawn_run(&l, async { Ok(()) }, RunOptions::default());
        tick().await;
        assert_eq!(l.depth(), 2);
        queued.abort();
        tick().await;
        assert_eq!(l.depth(), 1);
        gate.resolve();
        running.await.unwrap().unwrap();
        tick().await;
        assert_eq!((l.depth(), l.used()), (0, 0));
    }

    // describe("deadlines")

    #[tokio::test]
    async fn gives_up_on_the_callers_behalf_but_keeps_the_slot_until_the_work_really_ends() {
        // libvips cannot always be interrupted (measured: a single-frame AVIF encode
        // runs its full course with a timeout set). Releasing the slot at the
        // deadline would let a fresh caller start while the abandoned job still
        // holds its memory - the one thing the limiter exists to prevent.
        let l = limiter(1, 1);
        let mut stubborn = Gate::new();
        let wait = stubborn.wait();

        let job = l
            .run(
                move |_| async move {
                    wait.await;
                    Ok(())
                },
                RunOptions { timeout_ms: Some(10), ..Default::default() },
            )
            .await;
        assert_eq!(job.unwrap_err().downcast_ref::<TimeoutError>(), Some(&TimeoutError { ms: 10 }));

        // Still held: the work has not finished.
        assert_eq!(l.used(), 1);
        // ...and the deadline covers the WAIT as well, so a caller queued behind an
        // uninterruptible job is not stuck for as long as that job takes.
        let next = l.run(|_| async { Ok("next") }, RunOptions { timeout_ms: Some(10), ..Default::default() }).await;
        assert!(next.unwrap_err().is::<TimeoutError>());

        stubborn.resolve();
        tick().await;
        assert_eq!(l.used(), 0);
        assert_eq!(l.depth(), 0);
    }

    #[tokio::test]
    async fn returns_the_value_when_the_work_beats_the_deadline() {
        let l = limiter(1, 1);
        let r = l.run(|_| async { Ok("quick") }, RunOptions { timeout_ms: Some(1000), ..Default::default() }).await;
        assert_eq!(r.unwrap(), "quick");
        tick().await;
        assert_eq!(l.depth(), 0);
    }

    #[test]
    fn the_shared_limiter_is_sized_by_transform_cost() {
        let l = image_transforms();
        assert_eq!(l.shared.limit, TRANSFORM_CAPACITY);
        assert_eq!(l.shared.queue, TRANSFORM_QUEUE);
    }
}
