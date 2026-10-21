//! The planning-side reader of the resolution log. Pure and fast: no network, ever.

use super::book::ResolutionReader;
use super::outcome::{derive_first, MovePolicy, Naming, RetitleJudge};
use crate::provider::{
    ActionProviderError, ChangeBatch, ScrubActionProvider, SuggestionWithContext,
};
use crate::queue::{EditIntent, PendingRule};
use crate::subject::Subject;
use lastfm_edit::Track;

pub const IDENTITY_PROVIDER: &str = "musicbrainz_identity";

/// Bump when [`derive_first`] changes what it proposes, so resolved keys are re-planned.
const DERIVE_VERSION: u32 = 3;

pub struct IdentityProvider {
    reader: ResolutionReader,
    moves: MovePolicy,
    retitles: RetitleJudge,
}

impl IdentityProvider {
    pub fn new(reader: ResolutionReader, moves: MovePolicy, retitles: RetitleJudge) -> Self {
        Self {
            reader,
            moves,
            retitles,
        }
    }
}

#[async_trait::async_trait]
impl ScrubActionProvider for IdentityProvider {
    type Error = ActionProviderError;

    /// Per track: its key's `plannable()` outcomes; the first whose
    /// [`derive_edit`](super::outcome::derive_edit) yields a proposal becomes an `Edit`
    /// suggestion with the proposal's approval and rationale. Unresolved keys yield
    /// nothing (the change feed brings them back once resolved).
    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        _open_intents: Option<&[EditIntent]>,
        _pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        let snapshot = self.reader.snapshot();
        let naming = Naming {
            verdicts: &snapshot.naming,
            judge: self.retitles,
        };
        let mut results = Vec::new();
        for (idx, track) in tracks.iter().enumerate() {
            let subject = Subject {
                artist: track.artist.clone(),
                track: track.name.clone(),
                album: track.album.clone(),
                album_artist: track.album_artist.clone(),
            };
            let Some(entry) = snapshot.keys.get(&subject.key()) else {
                continue;
            };
            let proposal = derive_first(
                &subject,
                entry.plannable(),
                &entry.rejected,
                &self.moves,
                naming,
            )
            .and_then(|(_, derived)| derived.proposal);
            if let Some(proposal) = proposal {
                let suggestion = SuggestionWithContext::edit_with_confirmation(
                    proposal.edit,
                    proposal.approval.requires_confirmation(),
                    IDENTITY_PROVIDER.to_string(),
                )
                .with_rationale(proposal.rationale);
                results.push((idx, vec![suggestion]));
            }
        }
        Ok(results)
    }

    fn provider_name(&self) -> &str {
        IDENTITY_PROVIDER
    }

    fn generation(&self) -> Option<String> {
        Some(format!(
            "{} retitles={:?} derive={DERIVE_VERSION}",
            self.moves.fingerprint(),
            self.retitles
        ))
    }

    fn changes_since(&self, cursor: u64) -> Option<ChangeBatch> {
        let (keys, next_cursor) = self.reader.changes_since(cursor);
        Some(ChangeBatch { keys, next_cursor })
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
