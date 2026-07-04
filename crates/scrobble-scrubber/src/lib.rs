//! # scrobble-scrubber
//!
//! Automatic cleanup of Last.fm scrobble metadata, built on the [`scrobble_store`]
//! local mirror.
//!
//! Architecture: a fast, local **planner** turns scrobbles into durable *edit intents*
//! (via a pluggable provider stack — regex rewrite rules, MusicBrainz, LLM); a paced
//! **executor** drains the intent queue through the store's crash-safe
//! [`MirroredEditor`](scrobble_store::MirroredEditor), owning all rate-limited last.fm
//! write traffic. The two communicate only through durable state and broadcast events.

pub mod default_rules;
pub mod error;
pub mod events;
pub mod executor;
pub mod feed;
pub mod filters;
pub mod handle;
#[cfg(feature = "musicbrainz")]
pub mod musicbrainz;
#[cfg(feature = "openai")]
pub mod openai;
pub mod ops;
pub mod planner;
pub mod policy;
pub mod provider;
pub mod queue;
pub mod rewrite;
pub mod state;
pub mod subject;

pub use error::{Result, ScrubberError};
pub use events::{ExecReport, PlanReport, ScrubberEvent, ScrubberEventBus, ScrubberEventReceiver};
pub use executor::{Executor, ExecutorOptions};
pub use feed::{FeedBatch, ScrubFeed};
pub use filters::{ReleaseFilterConfig, ReleaseFilterType};
pub use handle::{bridge_sync_events, ScrubberActor, ScrubberCommand, ScrubberHandle};
#[cfg(feature = "musicbrainz")]
pub use musicbrainz::{
    CompilationToCanonicalProvider, MusicBrainzClient, MusicBrainzScrubActionProvider,
};
#[cfg(feature = "openai")]
pub use openai::{OpenAIProviderConfig, OpenAIScrubActionProvider, DEFAULT_SYSTEM_PROMPT};
pub use ops::{
    approve_intent, approve_pending_rule, reinstate_intent, reject_intent, reject_pending_rule,
};
pub use planner::{Planner, RULES_PROVIDER};
pub use policy::{EditDecision, Policy};
pub use provider::{
    ActionProviderError, BoxedProvider, OrScrubActionProvider, RewriteRulesScrubActionProvider,
    ScrubActionProvider, ScrubActionSuggestion, SuggestionWithContext,
};
pub use queue::{
    EditIntent, InstanceStatus, IntentState, PendingRule, PendingRuleState, QueueEvent,
    QueueEventKind, RuleEvent, RuleEventKind,
};
pub use rewrite::{
    any_rules_apply, any_rules_match, apply_all_rules, create_no_op_edit, default_rules,
    load_comprehensive_default_rules, RewriteError, RewriteRule, SdRule,
};
pub use state::{
    rules_hash, DismissedEntry, FsScrubberState, MemoryScrubberState, ProviderCoverage,
    ScrubberState,
};
pub use subject::{group_by_subject, Subject};
