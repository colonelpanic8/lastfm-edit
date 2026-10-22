//! Shared display components.

pub mod activity_log;
pub mod art;
pub mod execution_notice;
pub mod intent_card;

pub use activity_log::ActivityLog;
pub use art::Art;
pub use execution_notice::{ErrorNotice, ExecutionNotice};
pub use intent_card::{CardContext, IntentCard};
