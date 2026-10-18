//! The event-sourced resolution log (`<scrubber>/resolutions.jsonl`): what the lane has
//! concluded about each scrobble key.

use super::adjudicate::{case_hash, Case, CaseHash, Choice, Confidence, Verdict};
use super::mb::{RecordingId, ReleaseGroupId};
use super::naming::{naming_question, NamingJudge, NamingJudgement, NamingQuestion, NamingVerdict};
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
    /// An answer to the naming question of every key whose outcome retitles its album to
    /// `question`; the event's key is the one that raised it. Counts across resolver
    /// versions.
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

    /// Keys with a plannable outcome that raises `question`, sorted.
    fn keys_asking(&self, question: &NamingQuestion) -> Vec<ScrobbleKey> {
        let mut keys: Vec<ScrobbleKey> = self
            .keys
            .iter()
            .filter(|(key, entry)| {
                key.artist == question.artist
                    && entry.plannable().into_iter().any(|(outcome, _)| {
                        naming_question(&key.artist, key.album.as_deref(), outcome).as_ref()
                            == Some(question)
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

#[cfg(test)]
impl ResolutionReader {
    pub(crate) fn from_events(events: Vec<ResolutionEvent>) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(Arc::new(fold(events)))),
        }
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
mod tests {
    use super::*;
    use crate::identity::adjudicate::{CaseOption, OptionEvidence, OptionId};
    use crate::identity::canonical::Rung;
    use crate::identity::mb::{ArtistCredit, PrimaryType, ReleaseGroup, SecondaryType};
    use crate::identity::Canonical;
    use crate::identity::RESOLVER_VERSION;
    use crate::state::MemoryScrubberState;

    fn key(artist: &str, track: &str, album: &str) -> ScrobbleKey {
        ScrobbleKey {
            artist: artist.into(),
            track: track.into(),
            album: Some(album.into()),
        }
    }

    fn cocaine() -> ScrobbleKey {
        key("Eric Clapton", "Cocaine", "The Cream of Clapton")
    }

    fn group(id: &str, title: &str, secondary: Vec<SecondaryType>) -> ReleaseGroup {
        ReleaseGroup {
            id: ReleaseGroupId(id.into()),
            title: title.into(),
            primary: Some(PrimaryType::Album),
            secondary,
            first_release: None,
            credit: ArtistCredit::default(),
        }
    }

    /// Cocaine scrobbled on The Cream of Clapton, proposed onto `canonical`.
    fn outcome(recording: &str, canonical: &str) -> Outcome {
        Outcome {
            master: vec![RecordingId(recording.into())],
            title: "Cocaine".into(),
            scrobbled_group: Some(group(
                "cream",
                "The Cream of Clapton",
                vec![SecondaryType::Compilation],
            )),
            canonical: Some(Canonical {
                group: group(canonical, canonical, Vec::new()),
                rung: Rung::PlainAlbum,
                single_first: false,
            }),
            proof: Proof::SameRecording,
        }
    }

    fn case(options: &[(char, &str)]) -> Case {
        Case {
            key: cocaine(),
            plays: 12,
            siblings: Vec::new(),
            options: options
                .iter()
                .map(|(id, canonical)| CaseOption {
                    id: OptionId(*id),
                    outcome: outcome(id.to_string().as_str(), canonical),
                    evidence: OptionEvidence::default(),
                })
                .collect(),
        }
    }

    fn verdict(choice: Choice) -> Verdict {
        Verdict {
            choice,
            confidence: Confidence::High,
            reason: String::new(),
            invalid: false,
        }
    }

    fn target(album: &str) -> Target {
        Target {
            title: "Cocaine".into(),
            album: Some(album.into()),
            album_artist: Some("Eric Clapton".into()),
        }
    }

    fn event(at: u64, resolver: u32, kind: ResolutionEventKind) -> ResolutionEvent {
        ResolutionEvent {
            key: cocaine(),
            at,
            resolver,
            kind,
        }
    }

    fn resolved(at: u64, resolver: u32) -> ResolutionEvent {
        event(
            at,
            resolver,
            ResolutionEventKind::Resolved {
                outcome: outcome("40ae4c06", "Slowhand"),
            },
        )
    }

    fn not_found(at: u64, resolver: u32) -> ResolutionEvent {
        event(
            at,
            resolver,
            ResolutionEventKind::NotFound {
                queries: Vec::new(),
            },
        )
    }

    fn rejected(at: u64, resolver: u32, intent: u128, album: &str) -> ResolutionEvent {
        event(
            at,
            resolver,
            ResolutionEventKind::HumanRejected {
                intent: Uuid::from_u128(intent),
                target: target(album),
            },
        )
    }

    fn entry(status: KeyStatus, rejected: Vec<Target>, seq: u64, at: u64) -> KeyEntry {
        KeyEntry {
            status,
            rejected,
            seq,
            at,
            research: None,
        }
    }

    #[test]
    fn fold_takes_the_last_current_status_and_keeps_every_rejection() {
        let old = RESOLVER_VERSION - 1;
        let now = RESOLVER_VERSION;
        let settled = KeyStatus::Settled {
            outcome: outcome("40ae4c06", "Slowhand"),
        };
        let cases = [
            (
                "older version only",
                vec![resolved(10, old)],
                entry(KeyStatus::Stale, vec![], 1, 10),
            ),
            (
                "older version after a current one is ignored",
                vec![resolved(10, now), not_found(20, old)],
                entry(settled.clone(), vec![], 2, 10),
            ),
            (
                "last current status wins",
                vec![not_found(10, now), resolved(20, now)],
                entry(settled.clone(), vec![], 2, 20),
            ),
            (
                "deferral replaces a settlement",
                vec![
                    resolved(10, now),
                    event(
                        20,
                        now,
                        ResolutionEventKind::Deferred {
                            error: "503".into(),
                            retry_after: 99,
                        },
                    ),
                ],
                entry(KeyStatus::Deferred { retry_after: 99 }, vec![], 2, 20),
            ),
            (
                "rejections alone are stale",
                vec![rejected(10, now, 1, "Slowhand")],
                entry(KeyStatus::Stale, vec![target("Slowhand")], 1, 10),
            ),
            (
                "rejections count across versions, once per target",
                vec![
                    rejected(10, old, 1, "Slowhand"),
                    resolved(20, now),
                    rejected(30, now, 2, "Slowhand"),
                    rejected(40, now, 3, "Backless"),
                ],
                entry(settled, vec![target("Slowhand"), target("Backless")], 4, 20),
            ),
        ];
        for (name, events, expected) in cases {
            let snapshot = fold(events);
            assert_eq!(snapshot.keys.get(&cocaine()), Some(&expected), "{name}");
        }

        let snapshot = fold(vec![
            rejected(10, old, 1, "Slowhand"),
            rejected(20, now, 2, "Backless"),
        ]);
        assert_eq!(
            snapshot.consumed_rejections,
            HashSet::from([Uuid::from_u128(1), Uuid::from_u128(2)])
        );
    }

    #[test]
    fn verdicts_attach_only_to_the_open_case_they_were_given_for() {
        let open = case(&[('A', "Slowhand"), ('B', "Backless")]);
        let other = case(&[('A', "Slowhand"), ('B', "461 Ocean Boulevard")]);
        let adjudicated = |at, case: &Case, choice| {
            event(
                at,
                RESOLVER_VERSION,
                ResolutionEventKind::Adjudicated {
                    case_hash: case_hash(case),
                    verdict: verdict(choice),
                    by: "model".into(),
                    prompt_version: 1,
                    raw: String::new(),
                },
            )
        };
        let snapshot = fold(vec![
            event(
                10,
                RESOLVER_VERSION,
                ResolutionEventKind::Ambiguous {
                    case: open.clone(),
                    awaiting_llm: true,
                },
            ),
            adjudicated(20, &other, Choice::Keep),
            adjudicated(30, &open, Choice::Option(OptionId('B'))),
        ]);
        assert_eq!(
            snapshot.keys[&cocaine()].status,
            KeyStatus::Open {
                case: open,
                awaiting_llm: true,
                verdict: Some(verdict(Choice::Option(OptionId('B')))),
            }
        );
        assert_eq!(snapshot.adjudication_times, vec![20, 30]);
    }

    #[test]
    fn plannable_orders_outcomes_by_status_and_verdict() {
        let open = |awaiting_llm, choice: Option<Choice>| KeyStatus::Open {
            case: case(&[('A', "Slowhand"), ('B', "Backless"), ('C', "Unplugged")]),
            awaiting_llm,
            verdict: choice.map(verdict),
        };
        let cases = [
            (
                "settled",
                KeyStatus::Settled {
                    outcome: outcome("40ae4c06", "Slowhand"),
                },
                vec![("Slowhand", Chosen::Deterministic)],
            ),
            (
                "settled by research",
                KeyStatus::Settled {
                    outcome: Outcome {
                        proof: Proof::Researched {
                            by: "claude:opus".into(),
                        },
                        ..outcome("d31d4528", "Slowhand")
                    },
                },
                vec![("Slowhand", Chosen::Adjudicated)],
            ),
            (
                "llm picked B",
                open(true, Some(Choice::Option(OptionId('B')))),
                vec![
                    ("Backless", Chosen::Adjudicated),
                    ("Slowhand", Chosen::Adjudicated),
                    ("Unplugged", Chosen::Adjudicated),
                ],
            ),
            ("llm said keep", open(true, Some(Choice::Keep)), vec![]),
            (
                "llm unsure",
                open(true, Some(Choice::Unsure)),
                vec![
                    ("Slowhand", Chosen::Adjudicated),
                    ("Backless", Chosen::Adjudicated),
                    ("Unplugged", Chosen::Adjudicated),
                ],
            ),
            (
                "no llm",
                open(false, None),
                vec![
                    ("Slowhand", Chosen::Adjudicated),
                    ("Backless", Chosen::Adjudicated),
                    ("Unplugged", Chosen::Adjudicated),
                ],
            ),
            ("awaiting the llm", open(true, None), vec![]),
            ("not found", KeyStatus::NotFound, vec![]),
            ("deferred", KeyStatus::Deferred { retry_after: 9 }, vec![]),
            ("stale", KeyStatus::Stale, vec![]),
        ];
        for (name, status, expected) in cases {
            let entry = entry(status, vec![], 1, 1);
            let got: Vec<_> = entry
                .plannable()
                .into_iter()
                .map(|(outcome, chosen)| {
                    let canonical = outcome.canonical.as_ref().expect("canonical");
                    (canonical.group.title.as_str(), chosen)
                })
                .collect();
            assert_eq!(got, expected, "{name}");
        }
    }

    #[tokio::test]
    async fn changes_since_reports_each_changed_key_once_and_survives_reopen() {
        let state = Arc::new(MemoryScrubberState::new());
        let (mut writer, reader) = open(state.clone()).await.unwrap();
        assert_eq!(reader.changes_since(0), (vec![], 0));

        let horses = key("The Rolling Stones", "Wild Horses", "Sticky Fingers");
        let four = key("Wes Montgomery", "Four on Six", "Smokin' at the Half Note");
        let on = |key: &ScrobbleKey, at| ResolutionEvent {
            key: key.clone(),
            ..not_found(at, RESOLVER_VERSION)
        };
        writer
            .append(vec![on(&horses, 1), on(&four, 2), on(&horses, 3)])
            .await
            .unwrap();
        writer.append(vec![on(&four, 4)]).await.unwrap();

        assert_eq!(
            reader.changes_since(0),
            (vec![horses.clone(), four.clone()], 4)
        );
        assert_eq!(
            reader.changes_since(2),
            (vec![horses.clone(), four.clone()], 4)
        );
        assert_eq!(reader.changes_since(3), (vec![four.clone()], 4));
        assert_eq!(reader.changes_since(4), (vec![], 4));
        assert_eq!(
            reader.changes_since(9),
            (vec![], 9),
            "a cursor past the head belongs to another process's appends"
        );
        assert_eq!(
            reader.status_counts(),
            StatusCounts {
                not_found: 2,
                ..StatusCounts::default()
            }
        );

        let (_, reopened) = open(state).await.unwrap();
        assert_eq!(reopened.changes_since(3), (vec![four], 4));
        assert_eq!(reopened.get(&horses).map(|entry| entry.seq), Some(3));
    }

    fn researched(at: u64, resolver: u32, outcome: Option<Outcome>) -> ResolutionEvent {
        event(
            at,
            resolver,
            ResolutionEventKind::Researched {
                outcome,
                recording: Some(RecordingId("40ae4c06-3799-413b-b879-101dec39ac44".into())),
                release_group: None,
                confidence: Confidence::Medium,
                reason: "the Slowhand studio take".into(),
                by: "claude:opus".into(),
                raw: String::new(),
            },
        )
    }

    #[test]
    fn research_settles_the_key_only_when_musicbrainz_confirmed_it() {
        let found = Outcome {
            proof: Proof::Researched {
                by: "claude:opus".into(),
            },
            ..outcome("40ae4c06", "Slowhand")
        };
        let note = |at, verified| ResearchNote {
            at,
            by: "claude:opus".into(),
            confidence: Confidence::Medium,
            reason: "the Slowhand studio take".into(),
            recording: Some(RecordingId("40ae4c06-3799-413b-b879-101dec39ac44".into())),
            release_group: None,
            verified,
        };
        let now = RESOLVER_VERSION;
        let cases = [
            (
                "confirmed",
                vec![not_found(10, now), researched(20, now, Some(found.clone()))],
                KeyEntry {
                    research: Some(note(20, true)),
                    ..entry(
                        KeyStatus::Settled {
                            outcome: found.clone(),
                        },
                        vec![],
                        2,
                        20,
                    )
                },
            ),
            (
                "not confirmed",
                vec![not_found(10, now), researched(20, now, None)],
                KeyEntry {
                    research: Some(note(20, false)),
                    ..entry(KeyStatus::NotFound, vec![], 2, 10)
                },
            ),
            (
                "older resolver version",
                vec![not_found(10, now), researched(20, now - 1, Some(found))],
                entry(KeyStatus::NotFound, vec![], 2, 10),
            ),
        ];
        for (name, events, expected) in cases {
            let snapshot = fold(events);
            assert_eq!(snapshot.keys.get(&cocaine()), Some(&expected), "{name}");
            assert_eq!(snapshot.research_times, vec![20], "{name}");
            assert_eq!(snapshot.changes.last(), Some(&(2, cocaine())), "{name}");
        }
    }

    fn lennon(track: &str, album: &str) -> ScrobbleKey {
        key("John Lennon", track, album)
    }

    /// Settled on John Lennon/Plastic Ono Band, the release group `album` names.
    fn on_plastic_ono_band(track: &str, album: &str) -> ResolutionEvent {
        let group = group("8be83468", "John Lennon/Plastic Ono Band", Vec::new());
        ResolutionEvent {
            key: lennon(track, album),
            at: 10,
            resolver: RESOLVER_VERSION,
            kind: ResolutionEventKind::Resolved {
                outcome: Outcome {
                    master: vec![RecordingId(track.into())],
                    title: track.into(),
                    scrobbled_group: Some(group.clone()),
                    canonical: Some(Canonical {
                        group,
                        rung: Rung::PlainAlbum,
                        single_first: false,
                    }),
                    proof: Proof::SameRecording,
                },
            },
        }
    }

    fn plastic_ono_band() -> NamingQuestion {
        NamingQuestion {
            artist: "John Lennon".into(),
            album: "Plastic Ono Band".into(),
            title: "John Lennon/Plastic Ono Band".into(),
        }
    }

    fn judged(at: u64, verdict: NamingVerdict, by: NamingJudge) -> ResolutionEvent {
        ResolutionEvent {
            key: lennon("Mother", "Plastic Ono Band"),
            at,
            resolver: RESOLVER_VERSION,
            kind: ResolutionEventKind::NamingJudged {
                question: plastic_ono_band(),
                verdict,
                reason: "the album is released as Plastic Ono Band".into(),
                by,
                raw: String::new(),
            },
        }
    }

    #[test]
    fn a_naming_verdict_changes_every_key_on_the_album() {
        let llm = NamingJudge::Llm {
            model: "claude:opus".into(),
            prompt_version: 1,
        };
        let human = NamingJudge::Human {
            intent: Uuid::from_u128(7),
        };
        let reader = ResolutionReader::from_events(vec![
            on_plastic_ono_band("Mother", "Plastic Ono Band"),
            on_plastic_ono_band("Love", "Plastic Ono Band"),
            on_plastic_ono_band("Mother", "Plastic Ono Band (Remastered)"),
            on_plastic_ono_band("Love", "John Lennon/Plastic Ono Band"),
            resolved(10, RESOLVER_VERSION),
            judged(20, NamingVerdict::MusicBrainz, llm),
            judged(30, NamingVerdict::Keep, human.clone()),
        ]);

        assert_eq!(
            reader.changes_since(5),
            (
                vec![
                    lennon("Mother", "Plastic Ono Band"),
                    lennon("Love", "Plastic Ono Band"),
                    lennon("Mother", "Plastic Ono Band (Remastered)"),
                ],
                7
            )
        );
        let snapshot = reader.snapshot();
        assert_eq!(
            snapshot.naming.get(&plastic_ono_band()),
            Some(&NamingJudgement {
                verdict: NamingVerdict::Keep,
                reason: "the album is released as Plastic Ono Band".into(),
                by: human,
                at: 30,
            })
        );
        assert_eq!(snapshot.adjudication_times, vec![20]);
    }

    #[test]
    fn only_proven_settlements_confirm_rewrite_rules() {
        let isrc = |len_delta_ms| Proof::SharedIsrc {
            isrc: "NLF057790024".into(),
            len_delta_ms,
        };
        let cases = [
            ("same recording", Proof::SameRecording, Some(true)),
            ("shared ISRC, 2.1 s apart", isrc(2_107), Some(true)),
            ("shared ISRC, 5 s apart", isrc(5_000), Some(true)),
            ("shared ISRC, 5.001 s apart", isrc(5_001), None),
            ("same work", Proof::SameWork, None),
            ("title only", Proof::TitleOnly, None),
            (
                "researched",
                Proof::Researched {
                    by: "claude:opus".into(),
                },
                None,
            ),
        ];
        for (name, proof, expected) in cases {
            let settled = Outcome {
                proof,
                ..outcome("40ae4c06", "Slowhand")
            };
            let reader = ResolutionReader::from_events(vec![event(
                20,
                RESOLVER_VERSION,
                ResolutionEventKind::Resolved { outcome: settled },
            )]);
            assert_eq!(
                reader.settled_agrees(&cocaine(), Some("Cocaine"), Some("Slowhand")),
                expected,
                "{name}"
            );
        }

        let researched_outcome = Outcome {
            proof: Proof::Researched {
                by: "claude:opus".into(),
            },
            ..outcome("40ae4c06", "Slowhand")
        };
        let reader = ResolutionReader::from_events(vec![researched(
            20,
            RESOLVER_VERSION,
            Some(researched_outcome),
        )]);
        assert_eq!(
            reader.settled_agrees(&cocaine(), Some("Cocaine"), Some("Slowhand")),
            None,
            "settled by a research answer"
        );
    }
}
