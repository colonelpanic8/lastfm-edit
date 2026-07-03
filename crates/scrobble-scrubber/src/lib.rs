//! Rules engine for automatic editing of Last.fm scrobbles.
//!
//! `scrobble-scrubber` applies user-defined rules to scrobbles — for example,
//! stripping "Remastered" suffixes, normalizing artist names, or fixing album
//! metadata — and produces [`ScrobbleEdit`](lastfm_edit::ScrobbleEdit)s that can
//! be applied via the [`lastfm-edit`](lastfm_edit) client or against a local
//! [`ScrobbleStore`](scrobble_store::ScrobbleStore).
//!
//! This crate is currently a scaffold; the rule model and evaluation engine are
//! still being designed.

use lastfm_edit::Track;

/// Errors that can occur while evaluating or applying scrubbing rules.
#[derive(Debug, thiserror::Error)]
pub enum ScrubberError {
    /// Error originating from the underlying Last.fm client.
    #[error(transparent)]
    LastFm(#[from] lastfm_edit::LastFmError),

    /// Error originating from the scrobble store.
    #[error(transparent)]
    Store(#[from] scrobble_store::StoreError),
}

/// A single scrubbing rule that decides whether and how to edit a track.
///
/// This is a placeholder trait for the forthcoming rules engine.
pub trait Rule {
    /// A human-readable name for the rule.
    fn name(&self) -> &str;

    /// Whether this rule applies to the given track.
    fn matches(&self, track: &Track) -> bool;
}

/// Engine that evaluates a set of [`Rule`]s against tracks.
///
/// This is a placeholder for the forthcoming evaluation engine.
#[derive(Default)]
pub struct Scrubber {
    rules: Vec<Box<dyn Rule>>,
}

impl Scrubber {
    /// Create a new scrubber with no rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a rule with the scrubber.
    pub fn add_rule(&mut self, rule: Box<dyn Rule>) {
        self.rules.push(rule);
    }

    /// Number of registered rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the scrubber has no registered rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}
