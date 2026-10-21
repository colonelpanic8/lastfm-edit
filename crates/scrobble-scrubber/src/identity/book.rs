//! The event-sourced resolution log (`<scrubber>/resolutions.jsonl`): what the lane has
//! concluded about each scrobble key.

use super::adjudicate::{case_hash, Case, CaseHash, Choice, Confidence, Verdict};
use super::mb::{RecordingId, ReleaseGroupId};
use super::naming::{
    naming_questions, NamingJudge, NamingJudgement, NamingQuestion, NamingVerdict,
};
use super::outcome::{Chosen, Outcome, Proof, Target};
use crate::state::ScrubberState;
use crate::subject::ScrobbleKey;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// One line in `resolutions.jsonl`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolutionEvent {
    pub key: ScrobbleKey,
    pub at: u64,
    /// [`RESOLVER_VERSION`](super::RESOLVER_VERSION) that produced the event. Events from
    /// older versions fold to `Stale` (except human rejections, which always count).
    pub resolver: u32,
    #[serde(flatten)]
    pub kind: ResolutionEventKind,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ResolutionEventKind {
    /// The evidence allows exactly one outcome.
    Resolved {
        outcome: Outcome,
    },
    /// Several outcome-distinct options. `awaiting_llm` holds the human walk back until
    /// the LLM has answered.
    Ambiguous {
        case: Case,
        awaiting_llm: bool,
    },
    /// Audit record of one LLM answer for the key's open case.
    Adjudicated {
        case_hash: CaseHash,
        verdict: Verdict,
        by: String,
        prompt_version: u32,
        raw: String,
    },
    NotFound {
        queries: Vec<String>,
    },
    /// Transient failure; not retried before `retry_after` (unix seconds).
    Deferred {
        error: String,
        retry_after: u64,
    },
    /// The lane consumed a human rejection of identity intent `intent`.
    HumanRejected {
        intent: Uuid,
        target: Target,
    },
    /// Audit record of one LLM research answer. `outcome` is what MusicBrainz confirmed
    /// of it (it settles the key, for review); `None` leaves the status alone.
    Researched {
        outcome: Option<Outcome>,
        recording: Option<RecordingId>,
        release_group: Option<ReleaseGroupId>,
        confidence: Confidence,
        reason: String,
        by: String,
        raw: String,
    },
    /// An answer to the naming question of every key whose outcome retitles its album or
    /// track as `question` says; the event's key is the one that raised it. Counts across
    /// resolver versions.
    NamingJudged {
        question: NamingQuestion,
        verdict: NamingVerdict,
        reason: String,
        by: NamingJudge,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        raw: String,
    },
}

/// Folded state of one key.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyEntry {
    pub status: KeyStatus,
    /// Every target a human rejected for this key, across resolver versions.
    pub rejected: Vec<Target>,
    /// Seq of the last event for this key.
    pub seq: u64,
    /// When the status was last written (unix seconds).
    pub at: u64,
    /// The latest current-version research answer for this key.
    pub research: Option<ResearchNote>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResearchNote {
    /// Unix seconds.
    pub at: u64,
    pub by: String,
    pub confidence: Confidence,
    pub reason: String,
    pub recording: Option<RecordingId>,
    pub release_group: Option<ReleaseGroupId>,
    /// MusicBrainz confirmed the answer and it settled the key.
    pub verified: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyStatus {
    Settled {
        outcome: Outcome,
    },
    Open {
        case: Case,
        awaiting_llm: bool,
        verdict: Option<Verdict>,
    },
    NotFound,
    Deferred {
        retry_after: u64,
    },
    /// Only events from older resolver versions (or only human rejections) so far.
    Stale,
}

impl KeyEntry {
    /// Outcomes the planner may propose, in preference order. The provider proposes the
    /// first one whose edit isn't rejected and isn't a no-op.
    ///
    /// - Settled: its outcome; deterministic unless research found it.
    /// - Open + LLM picked an option: that option, then the rest (the human walk after a
    ///   rejection), all adjudicated.
    /// - Open + LLM said keep: nothing.
    /// - Open + LLM unsure, or no LLM: every option in order (the human walk).
    /// - Open + awaiting the LLM: nothing yet.
    pub(crate) fn plannable(&self) -> Vec<(&Outcome, Chosen)> {
        let (case, awaiting_llm, verdict) = match &self.status {
            KeyStatus::Settled { outcome } => {
                let chosen = match outcome.proof {
                    Proof::Researched { .. } => Chosen::Adjudicated,
                    _ => Chosen::Deterministic,
                };
                return vec![(outcome, chosen)];
            }
            KeyStatus::Open {
                case,
                awaiting_llm,
                verdict,
            } => (case, *awaiting_llm, verdict),
            KeyStatus::NotFound | KeyStatus::Deferred { .. } | KeyStatus::Stale => {
                return Vec::new()
            }
        };
        let picked = match verdict.as_ref().map(|verdict| &verdict.choice) {
            Some(Choice::Keep) => return Vec::new(),
            None if awaiting_llm => return Vec::new(),
            Some(Choice::Option(id)) => Some(*id),
            Some(Choice::Unsure) | None => None,
        };
        let mut options: Vec<_> = case.options.iter().collect();
        // Stable: the picked option first, the rest keep their order.
        options.sort_by_key(|option| Some(option.id) != picked);
        options
            .into_iter()
            .map(|option| (&option.outcome, Chosen::Adjudicated))
            .collect()
    }
}

/// Immutable folded view, shared behind an `Arc`.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Seq of the last event (1-based position in the log).
    pub head_seq: u64,
    pub keys: HashMap<ScrobbleKey, KeyEntry>,
    /// `(seq, key)` per event, ascending, for [`ResolutionReader::changes_since`].
    pub changes: Vec<(u64, ScrobbleKey)>,
    /// Queue intents whose rejection the lane already recorded.
    pub consumed_rejections: HashSet<Uuid>,
    /// Unix seconds of every LLM `Adjudicated` and `NamingJudged` event (daily LLM budget
    /// is derived from it).
    pub adjudication_times: Vec<u64>,
    /// Unix seconds of every `Researched` event (the daily research budget).
    pub research_times: Vec<u64>,
    /// The latest answer per naming question.
    pub naming: HashMap<NamingQuestion, NamingJudgement>,
}

/// Fold an ordered event stream. Last status event per key wins among current-version
/// events; `Adjudicated` attaches to the open case whose hash matches (else ignored with a
/// warning).
pub(crate) fn fold(events: impl IntoIterator<Item = ResolutionEvent>) -> Snapshot {
    let mut snapshot = Snapshot::default();
    for event in events {
        snapshot.apply(event);
    }
    snapshot
}

impl Snapshot {
    fn apply(&mut self, event: ResolutionEvent) {
        self.head_seq += 1;
        let seq = self.head_seq;
        self.changes.push((seq, event.key.clone()));
        let entry = self
            .keys
            .entry(event.key.clone())
            .or_insert_with(|| KeyEntry {
                status: KeyStatus::Stale,
                rejected: Vec::new(),
                seq,
                at: event.at,
                research: None,
            });
        entry.seq = seq;
        let current = event.resolver == super::RESOLVER_VERSION;
        let status = match event.kind {
            ResolutionEventKind::HumanRejected { intent, target } => {
                self.consumed_rejections.insert(intent);
                if !entry.rejected.contains(&target) {
                    entry.rejected.push(target);
                }
                return;
            }
            ResolutionEventKind::Adjudicated {
                case_hash: hash,
                verdict,
                ..
            } => {
                self.adjudication_times.push(event.at);
                match &mut entry.status {
                    KeyStatus::Open {
                        case,
                        verdict: slot,
                        ..
                    } if case_hash(case) == hash => {
                        *slot = Some(verdict);
                    }
                    _ => log::warn!(
                        "identity: verdict for {} matches no open case; ignored",
                        event.key
                    ),
                }
                return;
            }
            ResolutionEventKind::Researched {
                outcome,
                recording,
                release_group,
                confidence,
                reason,
                by,
                ..
            } => {
                self.research_times.push(event.at);
                if !current {
                    return;
                }
                entry.research = Some(ResearchNote {
                    at: event.at,
                    by,
                    confidence,
                    reason,
                    recording,
                    release_group,
                    verified: outcome.is_some(),
                });
                if let Some(outcome) = outcome {
                    entry.status = KeyStatus::Settled { outcome };
                    entry.at = event.at;
                }
                return;
            }
            ResolutionEventKind::NamingJudged {
                question,
                verdict,
                reason,
                by,
                ..
            } => {
                if matches!(by, NamingJudge::Llm { .. }) {
                    self.adjudication_times.push(event.at);
                }
                let asking = self.keys_asking(&question);
                self.changes
                    .extend(asking.into_iter().map(|key| (seq, key)));
                self.naming.insert(
                    question,
                    NamingJudgement {
                        verdict,
                        reason,
                        by,
                        at: event.at,
                    },
                );
                return;
            }
            _ if !current => {
                if entry.status == KeyStatus::Stale {
                    entry.at = event.at;
                }
                return;
            }
            ResolutionEventKind::Resolved { outcome } => KeyStatus::Settled { outcome },
            ResolutionEventKind::Ambiguous { case, awaiting_llm } => KeyStatus::Open {
                case,
                awaiting_llm,
                verdict: None,
            },
            ResolutionEventKind::NotFound { .. } => KeyStatus::NotFound,
            ResolutionEventKind::Deferred { retry_after, .. } => {
                KeyStatus::Deferred { retry_after }
            }
        };
        entry.status = status;
        entry.at = event.at;
    }

    /// The naming questions `key`'s plannable outcomes raise, with their answers.
    pub fn naming_questions(
        &self,
        key: &ScrobbleKey,
    ) -> Vec<(NamingQuestion, Option<&NamingJudgement>)> {
        let Some(entry) = self.keys.get(key) else {
            return Vec::new();
        };
        let mut questions: Vec<NamingQuestion> = entry
            .plannable()
            .into_iter()
            .flat_map(|(outcome, chosen)| naming_questions(key, outcome, chosen))
            .collect();
        questions.sort();
        questions.dedup();
        questions
            .into_iter()
            .map(|question| {
                let judgement = self.naming.get(&question);
                (question, judgement)
            })
            .collect()
    }

    /// Keys with a plannable outcome that raises `question`, sorted.
    fn keys_asking(&self, question: &NamingQuestion) -> Vec<ScrobbleKey> {
        let mut keys: Vec<ScrobbleKey> = self
            .keys
            .iter()
            .filter(|(key, entry)| {
                key.artist == question.artist
                    && entry.plannable().into_iter().any(|(outcome, chosen)| {
                        naming_questions(key, outcome, chosen).contains(question)
                    })
            })
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort();
        keys
    }
}

/// Cheap, cloneable read side for providers, rules, CLI and app.
#[derive(Clone)]
pub struct ResolutionReader {
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
}

impl ResolutionReader {
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().expect("snapshot lock").clone()
    }

    pub fn get(&self, key: &ScrobbleKey) -> Option<KeyEntry> {
        self.snapshot().keys.get(key).cloned()
    }

    pub fn status_counts(&self) -> StatusCounts {
        let mut counts = StatusCounts::default();
        for entry in self.snapshot().keys.values() {
            let count = match entry.status {
                KeyStatus::Settled { .. } => &mut counts.settled,
                KeyStatus::Open { .. } => &mut counts.open,
                KeyStatus::NotFound => &mut counts.not_found,
                KeyStatus::Deferred { .. } => &mut counts.deferred,
                KeyStatus::Stale => &mut counts.stale,
            };
            *count += 1;
        }
        counts
    }

    /// Keys with an event after `cursor`, deduped, and the new cursor (the head seq of the
    /// same snapshot). A cursor past the head reports nothing and is returned unchanged.
    pub fn changes_since(&self, cursor: u64) -> (Vec<ScrobbleKey>, u64) {
        let snapshot = self.snapshot();
        if cursor > snapshot.head_seq {
            // Another process appended since this one folded the log; whoever folds those
            // events plans them.
            return (Vec::new(), cursor);
        }
        let start = snapshot.changes.partition_point(|(seq, _)| *seq <= cursor);
        let mut seen = HashSet::new();
        let keys = snapshot.changes[start..]
            .iter()
            .filter(|(_, key)| seen.insert(key))
            .map(|(_, key)| key.clone())
            .collect();
        (keys, snapshot.head_seq)
    }

    /// Whether a rewrite agrees with the key's settled outcome under
    /// [`title::fold`](super::title::fold): `title` must equal the MB recording title and
    /// `album` the scrobbled or canonical release-group title. `None` (a field the
    /// rewrite leaves alone) always agrees. `None` overall unless the key is settled with
    /// a [proven](Proof::is_proven) outcome: same-work, title-only and researched matches
    /// vouch for nothing.
    pub(crate) fn settled_agrees(
        &self,
        key: &ScrobbleKey,
        title: Option<&str>,
        album: Option<&str>,
    ) -> Option<bool> {
        use super::title::fold as fold_text;

        let snapshot = self.snapshot();
        let KeyStatus::Settled { outcome } = &snapshot.keys.get(key)?.status else {
            return None;
        };
        if !outcome.proof.is_proven() {
            return None;
        }
        let title_agrees = title.is_none_or(|title| fold_text(title) == fold_text(&outcome.title));
        let movable = |canonical: &&super::canonical::Canonical| {
            outcome.scrobbled_group.as_ref().is_some_and(|scrobbled| {
                super::canonical::album_move_allowed(scrobbled, canonical).is_some()
            })
        };
        let album_agrees = album.is_none_or(|album| {
            let album = fold_text(album);
            outcome
                .scrobbled_group
                .iter()
                .chain(outcome.canonical.as_ref().filter(movable).map(|c| &c.group))
                .any(|group| fold_text(&group.title) == album)
        });
        Some(title_agrees && album_agrees)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub settled: u64,
    pub open: u64,
    pub not_found: u64,
    pub deferred: u64,
    pub stale: u64,
}

/// The single writer. Deliberately not `Clone`; only the lane holds it.
pub(crate) struct ResolutionWriter {
    state: Arc<dyn ScrubberState>,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
}

impl ResolutionWriter {
    /// Durably append, then publish a re-folded snapshot. A crash between the two is
    /// harmless: the next open folds from disk.
    pub async fn append(&mut self, events: Vec<ResolutionEvent>) -> crate::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        self.state.append_resolution_events(&events).await?;
        let current = self.snapshot.read().expect("snapshot lock").clone();
        let mut next = Snapshot::clone(&current);
        for event in events {
            next.apply(event);
        }
        *self.snapshot.write().expect("snapshot lock") = Arc::new(next);
        Ok(())
    }

    /// Re-folds the log and publishes it, picking up events other processes appended
    /// since [`open`]. Call it once this process holds the writer lock, before appending.
    pub async fn reload(&mut self) -> crate::Result<()> {
        let snapshot = fold(self.state.load_resolution_events().await?);
        *self.snapshot.write().expect("snapshot lock") = Arc::new(snapshot);
        Ok(())
    }

    pub fn reader(&self) -> ResolutionReader {
        ResolutionReader {
            snapshot: self.snapshot.clone(),
        }
    }
}

pub(crate) async fn open(
    state: Arc<dyn ScrubberState>,
) -> crate::Result<(ResolutionWriter, ResolutionReader)> {
    let snapshot = fold(state.load_resolution_events().await?);
    let writer = ResolutionWriter {
        state,
        snapshot: Arc::new(RwLock::new(Arc::new(snapshot))),
    };
    let reader = writer.reader();
    Ok((writer, reader))
}

#[cfg(test)]
#[path = "book_tests.rs"]
mod tests;
