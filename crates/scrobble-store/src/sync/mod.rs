//! Synchronization: the engine and its event vocabulary.

pub mod engine;
pub mod events;

pub use engine::{SyncEngine, SyncOptions, VerifyReport};
pub use events::{PauseReason, SyncEvent, SyncEventBus, SyncEventReceiver, SyncMode, SyncStats};
