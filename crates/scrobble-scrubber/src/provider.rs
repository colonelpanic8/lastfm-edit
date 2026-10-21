//! Action providers: pluggable sources of edit suggestions.
//!
//! Ported from the original scrobble-scrubber. A provider analyzes representative
//! [`Track`]s (one per [`Subject`](crate::subject::Subject)) and suggests immediate
//! edits, new rewrite rules, or nothing. Providers never touch last.fm — they are pure
//! analysis (network-using providers like MusicBrainz/OpenAI talk to *their* services,
//! not last.fm).

use crate::queue::{EditIntent, PendingRule};
use crate::rewrite::{RewriteError, RewriteRule};
use async_trait::async_trait;
use lastfm_edit::{ScrobbleEdit, Track};
use std::error::Error;
use std::fmt;

/// Generic error type for action providers
#[derive(Debug)]
pub struct ActionProviderError(pub String);

impl fmt::Display for ActionProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Action provider error: {}", self.0)
    }
}

impl Error for ActionProviderError {}

impl From<RewriteError> for ActionProviderError {
    fn from(err: RewriteError) -> Self {
        Self(format!("Rewrite error: {err}"))
    }
}

impl From<String> for ActionProviderError {
    fn from(msg: String) -> Self {
        Self(msg)
    }
}

impl From<&str> for ActionProviderError {
    fn from(msg: &str) -> Self {
        Self(msg.to_string())
    }
}

/// Represents a suggested action from a provider (rules engine, LLM, MusicBrainz, ...)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScrubActionSuggestion {
    /// Suggest an immediate scrobble edit (metadata-level; the executor expands it to
    /// exact per-instance edits)
    Edit(Box<ScrobbleEdit>),
    /// Propose a new rewrite rule
    ProposeRule {
        rule: Box<RewriteRule>,
        motivation: String,
    },
    /// No action needed
    NoAction,
}

/// Context wrapper for suggestions that includes confirmation requirements
#[derive(Debug, Clone)]
pub struct SuggestionWithContext {
    pub suggestion: ScrubActionSuggestion,
    pub requires_confirmation: bool,
    pub provider_name: String,
    /// Human-readable reason shown in review. Not machine-parsed.
    pub rationale: Option<String>,
}

impl SuggestionWithContext {
    pub fn new(
        suggestion: ScrubActionSuggestion,
        requires_confirmation: bool,
        provider_name: String,
    ) -> Self {
        Self {
            suggestion,
            requires_confirmation,
            provider_name,
            rationale: None,
        }
    }

    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }

    pub fn edit_with_confirmation(
        edit: ScrobbleEdit,
        requires_confirmation: bool,
        provider_name: String,
    ) -> Self {
        Self::new(
            ScrubActionSuggestion::Edit(Box::new(edit)),
            requires_confirmation,
            provider_name,
        )
    }

    pub fn propose_rule_with_confirmation(
        rule: RewriteRule,
        motivation: String,
        requires_confirmation: bool,
        provider_name: String,
    ) -> Self {
        Self::new(
            ScrubActionSuggestion::ProposeRule {
                rule: Box::new(rule),
                motivation,
            },
            requires_confirmation,
            provider_name,
        )
    }

    pub fn no_action(provider_name: String) -> Self {
        Self::new(ScrubActionSuggestion::NoAction, false, provider_name)
    }
}

/// Trait for providers that can suggest scrobble actions.
///
/// `Send` bound is deliberate: providers never hold the (!Send) lastfm-edit client, so
/// they remain usable from multi-threaded hosts (e.g. a future UI).
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async-trait adds must_use to generated futures"
)]
pub trait ScrubActionProvider: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Analyze multiple tracks and provide suggestions for improvements.
    /// Returns a vector of (track_index, suggestions) pairs.
    ///
    /// Optional context parameters help avoid duplicate suggestions:
    /// - `open_intents`: edit intents already queued (awaiting approval or execution)
    /// - `pending_rules`: rewrite rules already proposed and awaiting approval
    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        open_intents: Option<&[EditIntent]>,
        pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error>;

    /// Stable, filesystem-safe identifier for this provider (used for planning-coverage
    /// files and event attribution).
    fn provider_name(&self) -> &str;

    /// Bind rule-dependent analysis to the immutable rules loaded for a planning pass.
    fn rules_snapshot(&self, _rules: &[RewriteRule]) -> Option<BoxedProvider> {
        None
    }

    /// Durable change feed: keys whose analysis may differ since `cursor` because the
    /// provider's external evidence changed. The planner re-plans their live records
    /// (without a coverage claim) and persists `next_cursor` only after enqueueing.
    /// Providers whose answers depend only on the record have no feed.
    fn changes_since(&self, _cursor: u64) -> Option<ChangeBatch> {
        None
    }

    /// Identifies the provider's configuration. When it differs from the one its planning
    /// coverage was built under, the planner forgets that coverage and change cursor so
    /// everything is re-planned under the new configuration.
    fn generation(&self) -> Option<String> {
        None
    }
}

/// Keys a provider's change feed reports, and the cursor to resume after them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeBatch {
    pub keys: Vec<crate::subject::ScrobbleKey>,
    pub next_cursor: u64,
}

/// Rewrite rules-based action provider
pub struct RewriteRulesScrubActionProvider {
    rules: Vec<RewriteRule>,
    identity: Option<crate::identity::ResolutionReader>,
}

impl RewriteRulesScrubActionProvider {
    #[must_use]
    pub const fn from_rules(rules: Vec<RewriteRule>) -> Self {
        Self {
            rules,
            identity: None,
        }
    }

    /// Check `requires_musicbrainz_confirmation` rules against resolved identities. Until
    /// a key settles with proof its gated rules go to human review; the identity log's
    /// change feed then re-plans it, so a confirmation auto-applies and a contradiction
    /// drops the rule.
    #[must_use]
    pub fn with_identity(mut self, reader: crate::identity::ResolutionReader) -> Self {
        self.identity = Some(reader);
        self
    }

    pub fn rules(&self) -> &[RewriteRule] {
        &self.rules
    }

    /// Apply rules sequentially to a track, gating on per-rule MusicBrainz confirmation
    /// when requested. Returns Some((final_edit, requires_confirmation)) if any changes
    /// applied, otherwise None.
    async fn apply_rules_sequentially(
        &self,
        track: &Track,
    ) -> Result<Option<(ScrobbleEdit, bool)>, ActionProviderError> {
        let mut edit = crate::rewrite::create_no_op_edit(track);
        let mut any_changes = false;
        let mut requires_confirmation_applied = false;

        for rule in &self.rules {
            if !rule.matches_scrobble_edit(&edit)? {
                continue;
            }

            let mut candidate = edit.clone();
            let changed = rule.apply(&mut candidate)?;
            if !changed {
                continue;
            }

            if rule.requires_musicbrainz_confirmation {
                match self.musicbrainz_confirmation(track, &candidate) {
                    MbConfirmation::Confirmed => {}
                    MbConfirmation::Contradicted => {
                        log::debug!(
                            "Rewrite rule '{}' contradicted by MusicBrainz identity for '{} - {}'",
                            rule.name.as_deref().unwrap_or("Unnamed"),
                            track.artist,
                            track.name,
                        );
                        continue;
                    }
                    MbConfirmation::Unresolved | MbConfirmation::NoIdentity => {
                        requires_confirmation_applied = true;
                    }
                }
            }

            // Accept candidate
            edit = candidate;
            any_changes = true;
            requires_confirmation_applied |= rule.requires_confirmation;
        }

        if any_changes {
            Ok(Some((edit, requires_confirmation_applied)))
        } else {
            Ok(None)
        }
    }

    /// Whether the resolved identity of `track`'s key agrees with a rule's rewrite: the
    /// rewritten title fold-equals the MB recording title, and a rewritten album
    /// fold-equals the scrobbled or canonical release-group title. `Unresolved` (not yet
    /// settled, deferred, not found, or settled without proof) applies the rule for human
    /// review, as without identity.
    fn musicbrainz_confirmation(&self, track: &Track, candidate: &ScrobbleEdit) -> MbConfirmation {
        let Some(reader) = &self.identity else {
            return MbConfirmation::NoIdentity;
        };
        // Identity never vouches for artists.
        if candidate.artist_name != track.artist {
            return MbConfirmation::Contradicted;
        }
        let key = crate::subject::ScrobbleKey {
            artist: track.artist.clone(),
            track: track.name.clone(),
            album: track.album.clone(),
        };
        let title = candidate
            .track_name
            .as_deref()
            .filter(|title| *title != track.name);
        let album = candidate
            .album_name
            .as_deref()
            .filter(|album| track.album.as_deref() != Some(*album));
        match reader.settled_agrees(&key, title, album) {
            Some(true) => MbConfirmation::Confirmed,
            Some(false) => MbConfirmation::Contradicted,
            None => MbConfirmation::Unresolved,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MbConfirmation {
    Confirmed,
    Contradicted,
    Unresolved,
    /// No identity reader configured: a human confirms instead.
    NoIdentity,
}

#[async_trait]
impl ScrubActionProvider for RewriteRulesScrubActionProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        _open_intents: Option<&[EditIntent]>,
        _pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        let mut results = Vec::new();

        for (index, track) in tracks.iter().enumerate() {
            // Early continue if no rules apply
            if !crate::rewrite::any_rules_apply(&self.rules, track)? {
                continue;
            }

            // Apply rules with per-rule MB gating
            if let Some((final_edit, requires_confirmation)) =
                self.apply_rules_sequentially(track).await?
            {
                results.push((
                    index,
                    vec![SuggestionWithContext::edit_with_confirmation(
                        final_edit,
                        requires_confirmation,
                        self.provider_name().to_string(),
                    )],
                ));
            }
        }

        Ok(results)
    }

    fn provider_name(&self) -> &'static str {
        "rewrite_rules"
    }

    fn rules_snapshot(&self, rules: &[RewriteRule]) -> Option<BoxedProvider> {
        Some(boxed(Self {
            rules: rules.to_vec(),
            identity: self.identity.clone(),
        }))
    }

    fn changes_since(&self, cursor: u64) -> Option<ChangeBatch> {
        let (keys, next_cursor) = self.identity.as_ref()?.changes_since(cursor);
        Some(ChangeBatch { keys, next_cursor })
    }
}

/// Shared handles analyze like the provider they wrap (lets hosts keep an `Arc` for
/// inspection while the planner owns another).
#[async_trait]
impl<P> ScrubActionProvider for std::sync::Arc<P>
where
    P: ScrubActionProvider + Send + Sync + ?Sized,
{
    type Error = P::Error;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        open_intents: Option<&[EditIntent]>,
        pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        (**self)
            .analyze_tracks(tracks, open_intents, pending_rules)
            .await
    }

    fn provider_name(&self) -> &str {
        (**self).provider_name()
    }
    fn rules_snapshot(&self, rules: &[RewriteRule]) -> Option<BoxedProvider> {
        (**self).rules_snapshot(rules)
    }

    fn changes_since(&self, cursor: u64) -> Option<ChangeBatch> {
        (**self).changes_since(cursor)
    }

    fn generation(&self) -> Option<String> {
        (**self).generation()
    }
}

/// Adapter to convert different error types to the unified [`ActionProviderError`].
pub struct ErrorAdapter<P> {
    inner: P,
}

impl<P> ErrorAdapter<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<P> ScrubActionProvider for ErrorAdapter<P>
where
    P: ScrubActionProvider + Send + Sync,
    P::Error: Into<ActionProviderError>,
{
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        open_intents: Option<&[EditIntent]>,
        pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        self.inner
            .analyze_tracks(tracks, open_intents, pending_rules)
            .await
            .map_err(std::convert::Into::into)
    }

    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }
    fn rules_snapshot(&self, rules: &[RewriteRule]) -> Option<BoxedProvider> {
        self.inner.rules_snapshot(rules)
    }

    fn changes_since(&self, cursor: u64) -> Option<ChangeBatch> {
        self.inner.changes_since(cursor)
    }

    fn generation(&self) -> Option<String> {
        self.inner.generation()
    }
}

/// A boxed, error-unified provider — the shape the planner drives.
pub type BoxedProvider = Box<dyn ScrubActionProvider<Error = ActionProviderError>>;

/// Box a provider behind the unified error type.
pub fn boxed<P>(provider: P) -> BoxedProvider
where
    P: ScrubActionProvider + 'static,
    P::Error: Into<ActionProviderError>,
{
    Box::new(ErrorAdapter::new(provider))
}

/// Combines multiple providers, aggregating every provider's suggestions per track.
/// Provider errors are logged and skipped so one failing provider doesn't block others.
pub struct OrScrubActionProvider {
    providers: Vec<BoxedProvider>,
}

impl Default for OrScrubActionProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OrScrubActionProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add_provider<P>(mut self, provider: P) -> Self
    where
        P: ScrubActionProvider + 'static,
        P::Error: Into<ActionProviderError>,
    {
        self.providers.push(boxed(provider));
        self
    }
}

#[async_trait]
impl ScrubActionProvider for OrScrubActionProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        open_intents: Option<&[EditIntent]>,
        pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        let mut combined_results: Vec<(usize, Vec<SuggestionWithContext>)> = Vec::new();

        for provider in &self.providers {
            match provider
                .analyze_tracks(tracks, open_intents, pending_rules)
                .await
            {
                Ok(provider_results) => {
                    for (track_idx, suggestions) in provider_results {
                        if let Some(existing) = combined_results
                            .iter_mut()
                            .find(|(idx, _)| *idx == track_idx)
                        {
                            existing.1.extend(suggestions);
                        } else {
                            combined_results.push((track_idx, suggestions));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Error from provider '{}': {e}", provider.provider_name());
                }
            }
        }

        Ok(combined_results)
    }

    fn provider_name(&self) -> &'static str {
        "or_provider"
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
