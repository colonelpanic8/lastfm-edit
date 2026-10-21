//! What a resolution concludes, and the pure derivation of edits from it. The approval
//! policy lives here and nowhere else.

use super::canonical::{album_move_allowed, AlbumMove, Canonical, Rung};
use super::mb::{RecordingId, ReleaseGroup};
use super::naming::{
    classify_edition_fix, EditionFix, NamingJudgement, NamingQuestion, NamingVerdict,
};
use super::title::{
    classify_title_change, fold, parse_title, strip_edition, ParsedTitle, TitleChange,
};
use crate::rewrite::create_no_op_edit;
use crate::subject::Subject;
use lastfm_edit::ScrobbleEdit;
use std::collections::HashMap;

/// ISRC-linked duplicates further apart than this are plausibly different edits or
/// mislabeled, so they never auto-apply.
const AUTO_ISRC_LEN_TOLERANCE_MS: u32 = 5_000;

/// What to do with one kind of album move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveMode {
    /// Leave the album as scrobbled.
    Off,
    /// Propose the move for approval.
    Review,
    /// Apply proven moves without approval; weaker evidence still goes to review.
    Auto,
}

/// Which album moves identity proposes, per kind. Applied at planning time, so changing it
/// needs no MusicBrainz traffic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MovePolicy {
    /// Compilations and greatest-hits albums -> the album the recording first appeared on.
    pub compilations: MoveMode,
    /// Edition titles ("Sound & Color (Deluxe Edition)") -> the release group's title.
    pub editions: MoveMode,
    /// Singles and EPs -> the album.
    pub singles: MoveMode,
}

impl Default for MovePolicy {
    fn default() -> Self {
        Self {
            compilations: MoveMode::Auto,
            editions: MoveMode::Auto,
            singles: MoveMode::Review,
        }
    }
}

impl MovePolicy {
    pub fn mode(&self, kind: AlbumMove) -> MoveMode {
        match kind {
            AlbumMove::FromCompilation => self.compilations,
            AlbumMove::EditionToGroup => self.editions,
            AlbumMove::FromSingleOrEp => self.singles,
        }
    }

    /// Stable identifier of the policy: a change re-plans every resolved key.
    pub fn fingerprint(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Compact and self-contained: planning never needs the MB cache.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    /// Recordings proven to be one master; the first is the one matched for the scrobble.
    pub master: Vec<RecordingId>,
    /// MB recording title, verbatim. Identity writes it, or the scrobbled title's
    /// fold-equal base.
    pub title: String,
    /// The release group the scrobbled album string matched, if any.
    pub scrobbled_group: Option<ReleaseGroup>,
    pub canonical: Option<Canonical>,
    pub proof: Proof,
}

/// Why we believe the scrobble is this master.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Proof {
    /// The canonical release contains the scrobbled recording itself.
    SameRecording,
    /// Scrobbled and canonical recordings are unmerged MB duplicates sharing an ISRC.
    SharedIsrc { isrc: String, len_delta_ms: u32 },
    /// Same work, different recording, no ISRC link. Never auto.
    SameWork,
    /// No album to anchor on, or a search MusicBrainz truncated: title + artist only.
    /// Never auto.
    TitleOnly,
    /// An LLM with web search named the recording; MusicBrainz confirmed it plausibly is
    /// this scrobble. `by` is the model label. Never auto.
    Researched { by: String },
}

impl Proof {
    /// Strong enough to act on without a human.
    pub(crate) fn is_proven(&self) -> bool {
        match self {
            Proof::SameRecording => true,
            Proof::SharedIsrc { len_delta_ms, .. } => *len_delta_ms <= AUTO_ISRC_LEN_TOLERANCE_MS,
            Proof::SameWork | Proof::TitleOnly | Proof::Researched { .. } => false,
        }
    }
}

/// The fields an identity proposal sets. Human rejection keys on target equality.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Target {
    pub title: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
}

/// What one outcome derives for a subject.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Derived {
    pub proposal: Option<Proposal>,
    /// Unanswered naming questions part of the edit waits on; the lane asks them.
    pub waiting: Vec<NamingQuestion>,
    /// A naming verdict or wait shaped the edit, so the walk ends at this outcome even
    /// without a proposal.
    named: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Proposal {
    pub edit: ScrobbleEdit,
    pub target: Target,
    pub approval: Approval,
    pub rationale: String,
}

/// Auto can only be minted by [`approval`]; the private field makes that structural.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Approval {
    Auto(AutoProof),
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutoProof(());

impl Approval {
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Approval::Review)
    }
}

/// How the outcome was chosen, which bounds its approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chosen {
    /// The only outcome the evidence allowed.
    Deterministic,
    /// Picked among options by an LLM or the human walk. Always review.
    Adjudicated,
}

/// Who answers a retitle's naming question before anyone has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetitleJudge {
    /// The lane asks the LLM; a deterministic album retitle or an adjudicated track
    /// retitle waits for its answer.
    Llm,
    /// No LLM is configured: retitles go to review.
    Reviewer,
}

/// The naming answers a derivation consults.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Naming<'a> {
    pub verdicts: &'a HashMap<NamingQuestion, NamingJudgement>,
    pub judge: RetitleJudge,
}

/// What a proposal does to the title.
enum TitlePart<'a> {
    Unchanged,
    /// Packaging junk goes; the rest folds equal to the MB recording title.
    Packaging,
    /// To the MB recording title, as the verdict `naming` allowed.
    Retitle {
        naming: &'a NamingJudgement,
    },
    /// A retitle no verdict allowed: a person decides.
    RetitleForReview {
        naming: Option<&'a NamingJudgement>,
    },
    /// The user's title stays over MB's; only packaging junk goes.
    KeepTitle {
        naming: &'a NamingJudgement,
    },
    /// A retitle waiting for the lane to ask its question: the title stays.
    Waiting(NamingQuestion),
}

/// What a proposal does to the album.
enum AlbumPart<'a> {
    Unchanged,
    /// To the canonical release group's title. `naming` is the verdict that allowed a
    /// retitle.
    Move {
        kind: AlbumMove,
        canonical: &'a Canonical,
        naming: Option<&'a NamingJudgement>,
    },
    /// An edition retitle no verdict allowed: a person decides.
    RetitleForReview {
        canonical: &'a Canonical,
        naming: Option<&'a NamingJudgement>,
    },
    /// The user's title stays over the group's; only its edition suffix goes.
    KeepTitle {
        canonical: &'a Canonical,
        naming: &'a NamingJudgement,
    },
    /// The user's title, which has no edition suffix, stays over the group's.
    Kept {
        canonical: &'a Canonical,
        naming: &'a NamingJudgement,
    },
    /// An edition retitle waiting for the lane to ask its question: the album stays.
    Waiting(NamingQuestion),
}

/// Pure: subject + outcome -> at most one proposal, with the naming questions it waits on.
/// No proposal when nothing would change or a human rejected the target for this subject.
///
/// Title and album keep the user's spelling when, with the packaging or edition suffix
/// removed, it folds equal to the MB text. Moves off a compilation, single or EP set the
/// album artist to the canonical credit (the executor otherwise keeps "Various Artists");
/// an edition fix leaves it alone. An edition fix that retitles the album, and a title
/// change beyond packaging, follow their naming verdicts.
pub(crate) fn derive_edit(
    subject: &Subject,
    outcome: &Outcome,
    chosen: Chosen,
    rejected: &[Target],
    policy: &MovePolicy,
    naming: Naming<'_>,
) -> Derived {
    let parsed = parse_title(&subject.track);
    let title_part = title_part(subject, outcome, chosen, naming);
    let album = album_part(subject, outcome, chosen, policy, naming);

    let title = match &title_part {
        TitlePart::Unchanged | TitlePart::Waiting(_) => subject.track.clone(),
        TitlePart::Packaging | TitlePart::Retitle { .. } | TitlePart::RetitleForReview { .. } => {
            in_users_spelling(&parsed.base, &outcome.title)
        }
        TitlePart::KeepTitle { .. } => without_packaging(&parsed),
    };
    let moved = |canonical: &Canonical| {
        subject
            .album
            .as_deref()
            .map(|scrobbled| in_users_spelling(&strip_edition(scrobbled), &canonical.group.title))
    };
    let (album_name, album_artist) = match &album {
        AlbumPart::Unchanged | AlbumPart::Kept { .. } | AlbumPart::Waiting(_) => {
            (subject.album.clone(), subject.album_artist.clone())
        }
        AlbumPart::Move {
            kind, canonical, ..
        } => (
            moved(canonical),
            match kind {
                AlbumMove::EditionToGroup => subject.album_artist.clone(),
                AlbumMove::FromCompilation | AlbumMove::FromSingleOrEp => {
                    credited_album_artist(canonical, &subject.artist)
                        .or_else(|| subject.album_artist.clone())
                }
            },
        ),
        AlbumPart::RetitleForReview { canonical, .. } => {
            (moved(canonical), subject.album_artist.clone())
        }
        AlbumPart::KeepTitle { .. } => (
            subject.album.as_deref().map(strip_edition),
            subject.album_artist.clone(),
        ),
    };
    let target = Target {
        title,
        album: album_name,
        album_artist,
    };
    let unchanged = target.title == subject.track
        && target.album == subject.album
        && target.album_artist == subject.album_artist;
    let proposal = (!unchanged && !rejected.contains(&target)).then(|| {
        let mut edit = create_no_op_edit(&subject.representative_track(0, None));
        edit.track_name = Some(target.title.clone());
        edit.album_name = target.album.clone();
        edit.album_artist_name = target.album_artist.clone();
        Proposal {
            edit,
            approval: approval(chosen, &outcome.proof, &title_part, &album, policy),
            rationale: rationale(subject, outcome, chosen, &title_part, &album),
            target,
        }
    });
    let named = title_part.named() || album.named();
    let waiting = [title_part.waiting(), album.waiting()]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    Derived {
        proposal,
        waiting,
        named,
    }
}

/// The provider's walk over a key's plannable outcomes, in preference order: the first
/// that proposes an edit or whose edit a naming verdict or wait shaped. A rejected or
/// no-op outcome passes to the next (the human walk); a verdict about this outcome's
/// titles says nothing about the next one's.
pub(crate) fn derive_first<'a>(
    subject: &Subject,
    plannable: Vec<(&'a Outcome, Chosen)>,
    rejected: &[Target],
    policy: &MovePolicy,
    naming: Naming<'_>,
) -> Option<(&'a Outcome, Derived)> {
    plannable.into_iter().find_map(|(outcome, chosen)| {
        let derived = derive_edit(subject, outcome, chosen, rejected, policy, naming);
        (derived.proposal.is_some() || derived.named).then_some((outcome, derived))
    })
}

/// What `outcome` does to `track`. Only a pick among MusicBrainz options may change a
/// title beyond packaging; a deterministic or researched match may not drop "feat."
/// credits or version notes.
pub(crate) fn title_change(track: &str, outcome: &Outcome, chosen: Chosen) -> TitleChange {
    match classify_title_change(&parse_title(track), &outcome.title) {
        TitleChange::Other
            if chosen == Chosen::Deterministic
                || matches!(outcome.proof, Proof::Researched { .. }) =>
        {
            TitleChange::None
        }
        change => change,
    }
}

fn title_part<'a>(
    subject: &Subject,
    outcome: &Outcome,
    chosen: Chosen,
    naming: Naming<'a>,
) -> TitlePart<'a> {
    match title_change(&subject.track, outcome, chosen) {
        TitleChange::None => TitlePart::Unchanged,
        TitleChange::PackagingOnly => TitlePart::Packaging,
        TitleChange::Other => {
            let question = NamingQuestion::track(&subject.artist, &subject.track, &outcome.title);
            match naming.verdicts.get(&question) {
                Some(judgement) => match judgement.verdict {
                    NamingVerdict::MusicBrainz => TitlePart::Retitle { naming: judgement },
                    NamingVerdict::Keep => TitlePart::KeepTitle { naming: judgement },
                    NamingVerdict::Unsure => TitlePart::RetitleForReview {
                        naming: Some(judgement),
                    },
                },
                None if naming.judge == RetitleJudge::Llm => TitlePart::Waiting(question),
                None => TitlePart::RetitleForReview { naming: None },
            }
        }
    }
}

/// The user's title without its packaging junk, when that is all its suffixes are.
fn without_packaging(parsed: &ParsedTitle) -> String {
    let packaging_only =
        !parsed.packaging.is_empty() && parsed.identity.is_empty() && parsed.credits.is_empty();
    if packaging_only {
        &parsed.base
    } else {
        &parsed.raw
    }
    .to_string()
}

impl TitlePart<'_> {
    fn named(&self) -> bool {
        match self {
            TitlePart::Unchanged | TitlePart::Packaging => false,
            TitlePart::RetitleForReview { naming } => naming.is_some(),
            TitlePart::Retitle { .. } | TitlePart::KeepTitle { .. } | TitlePart::Waiting(_) => true,
        }
    }

    fn waiting(&self) -> Option<&NamingQuestion> {
        match self {
            TitlePart::Waiting(question) => Some(question),
            _ => None,
        }
    }
}

impl AlbumPart<'_> {
    fn named(&self) -> bool {
        match self {
            AlbumPart::Unchanged => false,
            AlbumPart::Move { naming, .. } | AlbumPart::RetitleForReview { naming, .. } => {
                naming.is_some()
            }
            AlbumPart::KeepTitle { .. } | AlbumPart::Kept { .. } | AlbumPart::Waiting(_) => true,
        }
    }

    fn waiting(&self) -> Option<&NamingQuestion> {
        match self {
            AlbumPart::Waiting(question) => Some(question),
            _ => None,
        }
    }
}

fn album_part<'a>(
    subject: &Subject,
    outcome: &'a Outcome,
    chosen: Chosen,
    policy: &MovePolicy,
    naming: Naming<'a>,
) -> AlbumPart<'a> {
    let Some((kind, canonical)) = album_move(subject.album.as_deref(), outcome)
        .filter(|(kind, _)| policy.mode(*kind) != MoveMode::Off)
    else {
        return AlbumPart::Unchanged;
    };
    let question = match (kind, subject.album.as_deref()) {
        (AlbumMove::EditionToGroup, Some(album)) => {
            match classify_edition_fix(&subject.artist, album, &canonical.group.title) {
                EditionFix::Retitle(question) => Some(question),
                EditionFix::SuffixOnly => None,
            }
        }
        _ => None,
    };
    let Some(question) = question else {
        return AlbumPart::Move {
            kind,
            canonical,
            naming: None,
        };
    };
    match naming.verdicts.get(&question) {
        Some(judgement) => match judgement.verdict {
            NamingVerdict::MusicBrainz => AlbumPart::Move {
                kind,
                canonical,
                naming: Some(judgement),
            },
            NamingVerdict::Keep
                if question.users_title != subject.album.as_deref().unwrap_or("") =>
            {
                AlbumPart::KeepTitle {
                    canonical,
                    naming: judgement,
                }
            }
            NamingVerdict::Keep => AlbumPart::Kept {
                canonical,
                naming: judgement,
            },
            NamingVerdict::Unsure => AlbumPart::RetitleForReview {
                canonical,
                naming: Some(judgement),
            },
        },
        // The lane asks about deterministic outcomes; until it answers, the album waits.
        None if naming.judge == RetitleJudge::Llm && chosen == Chosen::Deterministic => {
            AlbumPart::Waiting(question)
        }
        None => AlbumPart::RetitleForReview {
            canonical,
            naming: None,
        },
    }
}

/// The user's own text when it already says what MusicBrainz says, so typography
/// ("Who's" vs "Who\u{2019}s") never splits their scrobbles.
fn in_users_spelling(users: &str, mb: &str) -> String {
    if fold(users) == fold(mb) { users } else { mb }.to_string()
}

/// The canonical credit's main artists, each name in `artist`'s spelling where it folds
/// equal ("Jay\u{2010}Z & Alicia Keys" for a scrobble by "JAY-Z" credits "JAY-Z & Alicia Keys").
fn credited_album_artist(canonical: &Canonical, artist: &str) -> Option<String> {
    let credit = canonical
        .group
        .credit
        .main_artists(|name| in_users_spelling(artist, name));
    Some(in_users_spelling(artist, &credit)).filter(|credit| !credit.trim().is_empty())
}

/// The move `outcome` allows for a scrobble on `album`, before [`MovePolicy`].
pub(crate) fn album_move<'a>(
    album: Option<&str>,
    outcome: &'a Outcome,
) -> Option<(AlbumMove, &'a Canonical)> {
    let (Some(album), Some(scrobbled), Some(canonical)) =
        (album, &outcome.scrobbled_group, &outcome.canonical)
    else {
        return None;
    };
    if fold(album) == fold(&canonical.group.title) {
        return None;
    }
    album_move_allowed(scrobbled, canonical).map(|kind| (kind, canonical))
}

fn rationale(
    subject: &Subject,
    outcome: &Outcome,
    chosen: Chosen,
    title: &TitlePart,
    album: &AlbumPart,
) -> String {
    let mut lines = Vec::new();
    let retitled = || {
        format!(
            "Title: \"{}\" -> \"{}\" (MusicBrainz title)",
            subject.track, outcome.title
        )
    };
    match title {
        TitlePart::Unchanged | TitlePart::Waiting(_) => {}
        TitlePart::Packaging => lines.push(format!(
            "Title: \"{}\" -> \"{}\" (packaging stripped)",
            subject.track, outcome.title
        )),
        TitlePart::Retitle { naming } => {
            lines.push(retitled());
            lines.push(naming_line(naming));
        }
        TitlePart::RetitleForReview { naming } => {
            lines.push(retitled());
            lines.push(naming.map_or_else(
                || "Naming: the title changes beyond packaging; for review".to_string(),
                naming_line,
            ));
        }
        TitlePart::KeepTitle { naming } => {
            let kept = without_packaging(&parse_title(&subject.track));
            lines.push(if kept == subject.track {
                format!(
                    "Title: \"{kept}\" kept (MusicBrainz titles it \"{}\")",
                    outcome.title
                )
            } else {
                format!(
                    "Title: \"{}\" -> \"{kept}\" (packaging stripped; MusicBrainz titles it \"{}\")",
                    subject.track, outcome.title
                )
            });
            lines.push(naming_line(naming));
        }
    }
    let from = outcome
        .scrobbled_group
        .as_ref()
        .map_or_else(String::new, ReleaseGroup::describe);
    let moved = |kind: AlbumMove, canonical: &Canonical| {
        let why = match kind {
            AlbumMove::EditionToGroup => "edition title of the same release group",
            AlbumMove::FromCompilation => "off a compilation",
            AlbumMove::FromSingleOrEp => "off a single or EP",
        };
        format!("Album: {from} -> {} ({why})", canonical.group.describe())
    };
    match album {
        AlbumPart::Unchanged | AlbumPart::Waiting(_) => {}
        AlbumPart::Move {
            kind,
            canonical,
            naming,
        } => {
            lines.push(moved(*kind, canonical));
            lines.extend(naming.map(naming_line));
        }
        AlbumPart::RetitleForReview { canonical, naming } => {
            lines.push(moved(AlbumMove::EditionToGroup, canonical));
            lines.push(naming.map_or_else(
                || "Naming: the title changes beyond its edition suffix; for review".to_string(),
                naming_line,
            ));
        }
        AlbumPart::KeepTitle { canonical, naming } => {
            let album = subject.album.as_deref().unwrap_or_default();
            lines.push(format!(
                "Album: \"{album}\" -> \"{}\" (edition suffix stripped; MusicBrainz titles it \"{}\")",
                strip_edition(album),
                canonical.group.title
            ));
            lines.push(naming_line(naming));
        }
        AlbumPart::Kept { canonical, naming } => {
            lines.push(format!(
                "Album: \"{}\" kept (MusicBrainz titles it \"{}\")",
                subject.album.as_deref().unwrap_or_default(),
                canonical.group.title
            ));
            lines.push(naming_line(naming));
        }
    }
    lines.push(match &outcome.proof {
        Proof::SameRecording => "Proof: the same MusicBrainz recording".to_string(),
        Proof::SharedIsrc { isrc, len_delta_ms } => format!(
            "Proof: recordings share ISRC {isrc}, lengths {:.1} s apart",
            f64::from(*len_delta_ms) / 1000.0
        ),
        Proof::SameWork => "Proof: same work, different recording".to_string(),
        Proof::TitleOnly => "Proof: title and artist match only".to_string(),
        Proof::Researched { by } => {
            format!("Proof: found by LLM research ({by}), verified on MusicBrainz")
        }
    });
    if chosen == Chosen::Adjudicated {
        lines.push("Chosen among several MusicBrainz options".to_string());
    }
    lines.join("\n")
}

fn naming_line(judgement: &NamingJudgement) -> String {
    let verdict = match judgement.verdict {
        NamingVerdict::Keep => "keep your title",
        NamingVerdict::MusicBrainz => "use MusicBrainz's title",
        NamingVerdict::Unsure => "unsure",
    };
    format!(
        "Naming: {verdict} ({}): {}",
        judgement.by.describe(),
        judgement.reason
    )
}

fn approval(
    chosen: Chosen,
    proof: &Proof,
    title: &TitlePart,
    album: &AlbumPart,
    policy: &MovePolicy,
) -> Approval {
    let move_safe = |kind: AlbumMove, canonical: &Canonical| {
        let kind_safe = match kind {
            AlbumMove::EditionToGroup => true,
            AlbumMove::FromCompilation => !canonical
                .group
                .credit
                .main_artists(str::to_string)
                .trim()
                .is_empty(),
            AlbumMove::FromSingleOrEp => false,
        };
        kind_safe
            && policy.mode(kind) == MoveMode::Auto
            && canonical.rung == Rung::PlainAlbum
            && !canonical.single_first
    };
    let album_safe = match album {
        AlbumPart::Unchanged | AlbumPart::Kept { .. } | AlbumPart::Waiting(_) => true,
        AlbumPart::Move {
            kind, canonical, ..
        } => move_safe(*kind, canonical),
        AlbumPart::KeepTitle { canonical, .. } => move_safe(AlbumMove::EditionToGroup, canonical),
        AlbumPart::RetitleForReview { .. } => false,
    };
    let title_safe = !matches!(title, TitlePart::RetitleForReview { .. });
    if chosen == Chosen::Deterministic && proof.is_proven() && album_safe && title_safe {
        Approval::Auto(AutoProof(()))
    } else {
        Approval::Review
    }
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod tests;
