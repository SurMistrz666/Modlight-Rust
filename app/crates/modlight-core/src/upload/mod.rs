//! Uploads that outlive the moment they were made: the offline queue
//! (`ModlightUploadQueue`), its file (`ModlightQueueStore`), its items and their
//! sentences, transfer progress, and the ambient upload target.

pub mod item;
pub mod progress;
pub mod queue;
pub mod store;
pub mod target;
pub mod text;
pub mod transport;

pub use item::{QueueItem, QueueReason, QueueState};
pub use progress::ProgressManager;
pub use queue::{
    AccountSlot, QueueAccount, QueueEvent, QueueOutcome, QueueTransport, QuotaAnswer, TakeOver,
    TransferItem, TransferOutcome, TransferResult, UploadQueue,
};
pub use store::QueueStore;
