//! The web platform's `AbortController` / `AbortSignal`, which upstream's
//! media code takes for granted (`req.signal`, `imageTransforms.run(fn,
//! { signal })`, `runAvTool(..., { signal })`). Not an upstream module: it is
//! the vocabulary those modules are written in.
//!
//! A signal is aborted once, with a reason; later aborts are ignored, exactly
//! like the DOM's. In Rust "the caller went away" is usually a dropped
//! future rather than an event, so [`AbortController::abort_on_drop`] turns
//! one into the other for code that hands a signal to work it does not await
//! to the end.

use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

/// Why a signal was aborted (`signal.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// The caller went away (`AbortedError`).
    Aborted,
    /// A deadline of `ms` milliseconds passed (`TimeoutError`).
    TimedOut { ms: u64 },
}

struct Inner {
    token: CancellationToken,
    reason: OnceLock<AbortReason>,
}

/// `AbortSignal`: cheap to clone, observed by any number of tasks.
#[derive(Clone)]
pub struct AbortSignal {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for AbortSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbortSignal").field("reason", &self.reason()).finish()
    }
}

impl AbortSignal {
    /// `signal.aborted`.
    pub fn aborted(&self) -> bool {
        self.inner.reason.get().is_some()
    }

    /// `signal.reason`.
    pub fn reason(&self) -> Option<AbortReason> {
        self.inner.reason.get().copied()
    }

    /// Resolves once the signal is aborted (`addEventListener("abort")`).
    pub async fn cancelled(&self) {
        self.inner.token.cancelled().await;
    }
}

/// `AbortController`.
#[derive(Clone)]
pub struct AbortController {
    signal: AbortSignal,
}

impl Default for AbortController {
    fn default() -> Self {
        AbortController::new()
    }
}

impl AbortController {
    pub fn new() -> AbortController {
        AbortController {
            signal: AbortSignal { inner: Arc::new(Inner { token: CancellationToken::new(), reason: OnceLock::new() }) },
        }
    }

    pub fn signal(&self) -> AbortSignal {
        self.signal.clone()
    }

    /// `controller.abort()`: the caller went away.
    pub fn abort(&self) {
        self.abort_with(AbortReason::Aborted);
    }

    /// `controller.abort(reason)`. The first reason wins.
    pub fn abort_with(&self, reason: AbortReason) {
        if self.signal.inner.reason.set(reason).is_ok() {
            self.signal.inner.token.cancel();
        }
    }

    /// A guard that aborts this controller when dropped, unless
    /// [`AbortOnDrop::disarm`] was called: hold it in a request handler and
    /// a client that hangs up (the handler future is dropped) aborts the
    /// signal the way `req.signal` does upstream.
    pub fn abort_on_drop(&self) -> AbortOnDrop {
        AbortOnDrop { controller: Some(self.clone()) }
    }
}

/// See [`AbortController::abort_on_drop`].
pub struct AbortOnDrop {
    controller: Option<AbortController>,
}

impl AbortOnDrop {
    pub fn disarm(mut self) {
        self.controller = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(c) = self.controller.take() {
            c.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_first_reason_wins_and_listeners_wake() {
        let c = AbortController::new();
        let s = c.signal();
        assert!(!s.aborted());
        let waiter = tokio::spawn({
            let s = s.clone();
            async move { s.cancelled().await }
        });
        c.abort_with(AbortReason::TimedOut { ms: 5 });
        c.abort();
        waiter.await.unwrap();
        assert!(s.aborted());
        assert_eq!(s.reason(), Some(AbortReason::TimedOut { ms: 5 }));
    }

    #[test]
    fn a_dropped_guard_aborts_and_a_disarmed_one_does_not() {
        let c = AbortController::new();
        c.abort_on_drop().disarm();
        assert!(!c.signal().aborted());
        drop(c.abort_on_drop());
        assert_eq!(c.signal().reason(), Some(AbortReason::Aborted));
    }
}
