//! Work off the UI thread: a tokio runtime whose tasks finish by handing the UI a
//! closure to run on the next frame.

use std::future::Future;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::state::AppState;

/// What a background task gives back: a change to make on the UI thread.
pub type Completion = Box<dyn FnOnce(&mut AppState, &egui::Context) + Send>;

pub struct Runtime {
    runtime: Option<tokio::runtime::Runtime>,
    sender: Sender<Completion>,
    receiver: Receiver<Completion>,
    ctx: Mutex<Option<egui::Context>>,
}

impl Runtime {
    pub fn new() -> Runtime {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("modlight-worker")
            .enable_all()
            .build()
            .ok();
        let (sender, receiver) = channel();
        Runtime {
            runtime,
            sender,
            receiver,
            ctx: Mutex::new(None),
        }
    }

    /// Tell the runtime which context to wake when work finishes.
    pub fn attach(&self, ctx: &egui::Context) {
        *self.ctx.lock().unwrap() = Some(ctx.clone());
    }

    /// Run `work` in the background; its completion runs on the UI thread.
    pub fn spawn<F, C>(&self, work: F)
    where
        F: Future<Output = C> + Send + 'static,
        C: FnOnce(&mut AppState, &egui::Context) + Send + 'static,
    {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let sender = self.sender.clone();
        let ctx = self.ctx.lock().unwrap().clone();
        runtime.spawn(async move {
            let completion: Completion = Box::new(work.await);
            let _ = sender.send(completion);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// Run a blocking job (decoding) on the runtime's blocking pool.
    pub fn spawn_blocking<F, C>(&self, work: F)
    where
        F: FnOnce() -> C + Send + 'static,
        C: FnOnce(&mut AppState, &egui::Context) + Send + 'static,
    {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let sender = self.sender.clone();
        let ctx = self.ctx.lock().unwrap().clone();
        runtime.spawn_blocking(move || {
            let completion: Completion = Box::new(work());
            let _ = sender.send(completion);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }

    /// The completions that arrived since the last frame.
    pub fn drain(&self) -> Vec<Completion> {
        self.receiver.try_iter().collect()
    }

    /// Block on a future (startup work only).
    pub fn block_on<F: Future>(&self, future: F) -> Option<F::Output> {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.block_on(future))
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime::new()
    }
}
