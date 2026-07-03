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
pub mod filters;
pub mod rewrite;

pub use filters::{ReleaseFilterConfig, ReleaseFilterType};
pub use rewrite::{
    any_rules_apply, any_rules_match, apply_all_rules, create_no_op_edit, default_rules,
    load_comprehensive_default_rules, RewriteError, RewriteRule, SdRule,
};
