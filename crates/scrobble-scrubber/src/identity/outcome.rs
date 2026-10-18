//! What a resolution concludes, and the pure derivation of edits from it. The approval
//! policy lives here and nowhere else.

use super::canonical::{album_move_allowed, AlbumMove, Canonical, Rung};
use super::mb::{RecordingId, ReleaseGroup};
use super::naming::{
    classify_edition_fix, EditionFix, NamingJudgement, NamingQuestion, NamingVerdict,
};
use super::title::{classify_title_change, fold, parse_title, strip_edition, TitleChange};
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
    /// The lane asks the LLM; a deterministic retitle waits for its answer.
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
    /// An edition retitle waiting for the lane to ask its question: the album stays.
    Waiting(NamingQuestion),
}

/// Pure: subject + outcome -> at most one proposal, with the naming questions it waits on.
/// No proposal when nothing would change or a human rejected the target for this subject.
///
/// Title and album keep the user's spelling when, with the packaging or edition suffix
/// removed, it folds equal to the MB text. Moves off a compilation, single or EP set the
/// album artist to the canonical credit (the executor otherwise keeps "Various Artists");
/// an edition fix leaves it alone. An edition fix that retitles the album follows its
/// naming verdict.
pub(crate) fn derive_edit(
    subject: &Subject,
    outcome: &Outcome,
    chosen: Chosen,
    rejected: &[Target],
    policy: &MovePolicy,
    naming: Naming<'_>,
) -> Derived {
    let parsed = parse_title(&subject.track);
    let title_change = match classify_title_change(&parsed, &outcome.title) {
        // Only a pick among MusicBrainz options may rewrite a title beyond packaging; a
        // deterministic or researched match may not drop "feat." credits or version notes.
        TitleChange::Other
            if chosen == Chosen::Deterministic
                || matches!(outcome.proof, Proof::Researched { .. }) =>
        {
            TitleChange::None
        }
        change => change,
    };
    let album = album_part(subject, outcome, chosen, policy, naming);

    let title = match title_change {
        TitleChange::None => subject.track.clone(),
        TitleChange::PackagingOnly | TitleChange::Other => {
            in_users_spelling(&parsed.base, &outcome.title)
        }
    };
    let moved = |canonical: &Canonical| {
        subject
            .album
            .as_deref()
            .map(|scrobbled| in_users_spelling(&strip_edition(scrobbled), &canonical.group.title))
    };
    let (album_name, album_artist) = match &album {
        AlbumPart::Unchanged | AlbumPart::Waiting(_) => {
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
            approval: approval(chosen, &outcome.proof, title_change, &album, policy),
            rationale: rationale(subject, outcome, chosen, title_change, &album),
            target,
        }
    });
    let waiting = match album {
        AlbumPart::Waiting(question) => vec![question],
        _ => Vec::new(),
    };
    Derived { proposal, waiting }
}

/// The provider's walk over a key's plannable outcomes, in preference order: the first
/// that proposes an edit, or failing that the first that waits on a naming question. A
/// rejected or no-op outcome passes to the next (the human walk).
pub(crate) fn derive_first<'a>(
    subject: &Subject,
    plannable: Vec<(&'a Outcome, Chosen)>,
    rejected: &[Target],
    policy: &MovePolicy,
    naming: Naming<'_>,
) -> Option<(&'a Outcome, Derived)> {
    let mut waiting = None;
    for (outcome, chosen) in plannable {
        let derived = derive_edit(subject, outcome, chosen, rejected, policy, naming);
        if derived.proposal.is_some() {
            return Some((outcome, derived));
        }
        if waiting.is_none() && !derived.waiting.is_empty() {
            waiting = Some((outcome, derived));
        }
    }
    waiting
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
            NamingVerdict::Keep if question.album != subject.album.as_deref().unwrap_or("") => {
                AlbumPart::KeepTitle {
                    canonical,
                    naming: judgement,
                }
            }
            NamingVerdict::Keep => AlbumPart::Unchanged,
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
    title: TitleChange,
    album: &AlbumPart,
) -> String {
    let mut lines = Vec::new();
    match title {
        TitleChange::None => {}
        TitleChange::PackagingOnly => lines.push(format!(
            "Title: \"{}\" -> \"{}\" (packaging stripped)",
            subject.track, outcome.title
        )),
        TitleChange::Other => lines.push(format!(
            "Title: \"{}\" -> \"{}\" (MusicBrainz title)",
            subject.track, outcome.title
        )),
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
    title: TitleChange,
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
        AlbumPart::Unchanged | AlbumPart::Waiting(_) => true,
        AlbumPart::Move {
            kind, canonical, ..
        } => move_safe(*kind, canonical),
        AlbumPart::KeepTitle { canonical, .. } => move_safe(AlbumMove::EditionToGroup, canonical),
        AlbumPart::RetitleForReview { .. } => false,
    };
    let title_safe = matches!(title, TitleChange::None | TitleChange::PackagingOnly);
    if chosen == Chosen::Deterministic && proof.is_proven() && album_safe && title_safe {
        Approval::Auto(AutoProof(()))
    } else {
        Approval::Review
    }
}

#[cfg(test)]
impl Naming<'static> {
    /// No LLM and no verdicts.
    pub(crate) fn reviewer() -> Self {
        Self {
            verdicts: Box::leak(Box::default()),
            judge: RetitleJudge::Reviewer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::canonical::examples::*;
    use super::super::mb::{ArtistCredit, PrimaryType, SecondaryType};
    use super::super::naming::NamingJudge;
    use super::*;

    const ISRC: &str = "NLF057790024";

    fn subject(artist: &str, track: &str, album: &str, album_artist: Option<&str>) -> Subject {
        Subject {
            artist: artist.to_string(),
            track: track.to_string(),
            album: Some(album.to_string()),
            album_artist: album_artist.map(str::to_string),
        }
    }

    fn outcome(
        title: &str,
        scrobbled: ReleaseGroup,
        canonical: Option<Canonical>,
        proof: Proof,
    ) -> Outcome {
        Outcome {
            master: vec![
                RecordingId("aca688be-742b-4c44-96b9-2278bdc483c3".to_string()),
                RecordingId("40ae4c06-3799-413b-b879-101dec39ac44".to_string()),
            ],
            title: title.to_string(),
            scrobbled_group: Some(scrobbled),
            canonical,
            proof,
        }
    }

    fn plain_album(group: ReleaseGroup) -> Option<Canonical> {
        Some(Canonical {
            group,
            rung: Rung::PlainAlbum,
            single_first: false,
        })
    }

    fn shared_isrc(len_delta_ms: u32) -> Proof {
        Proof::SharedIsrc {
            isrc: ISRC.to_string(),
            len_delta_ms,
        }
    }

    fn target(title: &str, album: &str, album_artist: Option<&str>) -> Target {
        Target {
            title: title.to_string(),
            album: Some(album.to_string()),
            album_artist: album_artist.map(str::to_string),
        }
    }

    fn cocaine_on_cream() -> (Subject, Outcome) {
        (
            subject(
                "Eric Clapton",
                "Cocaine",
                "The Cream of Clapton",
                Some("Various Artists"),
            ),
            outcome(
                "Cocaine",
                cream_of_clapton(),
                plain_album(slowhand()),
                shared_isrc(2_107),
            ),
        )
    }

    #[test]
    fn cocaine_moves_off_the_compilation_to_slowhand_automatically() {
        let (subject, outcome) = cocaine_on_cream();
        let proposal = derive_edit(
            &subject,
            &outcome,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal
        .unwrap();
        assert_eq!(
            proposal.edit,
            ScrobbleEdit {
                track_name_original: Some("Cocaine".to_string()),
                album_name_original: Some("The Cream of Clapton".to_string()),
                artist_name_original: "Eric Clapton".to_string(),
                album_artist_name_original: Some("Various Artists".to_string()),
                track_name: Some("Cocaine".to_string()),
                album_name: Some("Slowhand".to_string()),
                artist_name: "Eric Clapton".to_string(),
                album_artist_name: Some("Eric Clapton".to_string()),
                timestamp: None,
                edit_all: true,
            }
        );
        assert_eq!(
            proposal.target,
            target("Cocaine", "Slowhand", Some("Eric Clapton"))
        );
        assert!(!proposal.approval.requires_confirmation());
        assert_eq!(
            proposal.rationale,
            "Album: The Cream of Clapton (Album + Compilation, 1995) -> Slowhand (Album, 1977) (off a compilation)\n\
             Proof: recordings share ISRC NLF057790024, lengths 2.1 s apart"
        );
    }

    #[test]
    fn edition_fixes_change_only_the_edition_wording() {
        let alabama_shakes = || credit(&[("alabama-shakes", "Alabama Shakes", "")]);
        let sound_and_color = ReleaseGroup {
            credit: ArtistCredit::default(),
            ..group(
                "sound-and-color",
                "Sound & Color",
                PrimaryType::Album,
                &[],
                "2015-04-21",
                alabama_shakes(),
            )
        };
        let whos_next = group(
            "whos-next",
            "Who\u{2019}s Next",
            PrimaryType::Album,
            &[],
            "1971-08-14",
            credit(&[("the-who", "The Who", "")]),
        );
        let cases = [
            (
                subject(
                    "Alabama Shakes",
                    "Gimme All Your Love",
                    "Sound & Color (Deluxe Edition)",
                    Some("Alabama Shakes"),
                ),
                "Gimme All Your Love",
                sound_and_color,
                target(
                    "Gimme All Your Love",
                    "Sound & Color",
                    Some("Alabama Shakes"),
                ),
            ),
            (
                subject(
                    "The Who",
                    "Won't Get Fooled Again - Remastered",
                    "Who's Next (Deluxe Edition)",
                    None,
                ),
                "Won\u{2019}t Get Fooled Again",
                whos_next,
                target("Won't Get Fooled Again", "Who's Next", None),
            ),
        ];
        for (subject, title, group, expected) in cases {
            let outcome = Outcome {
                master: vec![],
                title: title.to_string(),
                scrobbled_group: Some(group.clone()),
                canonical: plain_album(group),
                proof: Proof::SameRecording,
            };
            let proposal = derive_edit(
                &subject,
                &outcome,
                Chosen::Deterministic,
                &[],
                &MovePolicy::default(),
                Naming::reviewer(),
            )
            .proposal
            .unwrap();
            assert_eq!(
                (proposal.target, proposal.approval.requires_confirmation()),
                (expected, false),
                "{}",
                subject.track
            );
        }
    }

    #[test]
    fn edition_retitles_follow_the_albums_naming_verdict() {
        let group = group(
            "8be83468-4917-390d-82b6-d890f058cd22",
            "John Lennon/Plastic Ono Band",
            PrimaryType::Album,
            &[],
            "1970-12-11",
            credit(&[("john-lennon", "John Lennon", "")]),
        );
        let question = NamingQuestion {
            artist: "John Lennon".into(),
            album: "Plastic Ono Band".into(),
            title: "John Lennon/Plastic Ono Band".into(),
        };
        let judged = |verdict| {
            HashMap::from([(
                question.clone(),
                NamingJudgement {
                    verdict,
                    reason: "Restores the official title.".into(),
                    by: NamingJudge::Llm {
                        model: "claude:opus".into(),
                        prompt_version: 1,
                    },
                    at: 1,
                },
            )])
        };
        let no_verdicts = HashMap::new();
        let musicbrainz = judged(NamingVerdict::MusicBrainz);
        let keep = judged(NamingVerdict::Keep);
        let unsure = judged(NamingVerdict::Unsure);
        let retitled = |track: &str| target(track, "John Lennon/Plastic Ono Band", None);

        let cases = [
            (
                "musicbrainz: retitled automatically",
                "Mother",
                "Plastic Ono Band",
                &musicbrainz,
                RetitleJudge::Llm,
                Some((retitled("Mother"), false)),
            ),
            (
                "keep: no album change",
                "Mother",
                "Plastic Ono Band",
                &keep,
                RetitleJudge::Llm,
                None,
            ),
            (
                "keep: the edition suffix still goes",
                "Mother",
                "Plastic Ono Band (Remastered)",
                &keep,
                RetitleJudge::Llm,
                Some((target("Mother", "Plastic Ono Band", None), false)),
            ),
            (
                "keep: the rest of the edit stands",
                "Mother - Remastered 2010",
                "Plastic Ono Band",
                &keep,
                RetitleJudge::Llm,
                Some((target("Mother", "Plastic Ono Band", None), false)),
            ),
            (
                "unsure: for review",
                "Mother",
                "Plastic Ono Band",
                &unsure,
                RetitleJudge::Llm,
                Some((retitled("Mother"), true)),
            ),
            (
                "no verdict and no LLM: for review",
                "Mother",
                "Plastic Ono Band",
                &no_verdicts,
                RetitleJudge::Reviewer,
                Some((retitled("Mother"), true)),
            ),
            (
                "no verdict yet: the album waits for the LLM",
                "Mother",
                "Plastic Ono Band",
                &no_verdicts,
                RetitleJudge::Llm,
                None,
            ),
            (
                "no verdict yet: the title fix goes ahead",
                "Mother - Remastered 2010",
                "Plastic Ono Band",
                &no_verdicts,
                RetitleJudge::Llm,
                Some((target("Mother", "Plastic Ono Band", None), false)),
            ),
        ];
        for (name, track, album, verdicts, judge, expected) in cases {
            let outcome = outcome(
                "Mother",
                group.clone(),
                plain_album(group.clone()),
                Proof::SameRecording,
            );
            let actual = derive_edit(
                &subject("John Lennon", track, album, None),
                &outcome,
                Chosen::Deterministic,
                &[],
                &MovePolicy::default(),
                Naming { verdicts, judge },
            )
            .proposal
            .map(|p| (p.target, p.approval.requires_confirmation()));
            assert_eq!(actual, expected, "{name}");
        }

        let proposal = derive_edit(
            &subject("John Lennon", "Mother", "Plastic Ono Band", None),
            &outcome(
                "Mother",
                group.clone(),
                plain_album(group),
                Proof::SameRecording,
            ),
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming {
                verdicts: &musicbrainz,
                judge: RetitleJudge::Llm,
            },
        )
        .proposal
        .unwrap();
        assert_eq!(
            proposal.rationale,
            "Album: John Lennon/Plastic Ono Band (Album, 1970) -> John Lennon/Plastic Ono Band (Album, 1970) (edition title of the same release group)\n\
             Naming: use MusicBrainz's title (claude:opus): Restores the official title.\n\
             Proof: the same MusicBrainz recording"
        );
    }

    #[test]
    fn compilation_move_without_a_canonical_credit_keeps_the_album_artist_for_review() {
        let (subject, mut outcome) = cocaine_on_cream();
        outcome.canonical.as_mut().unwrap().group.credit = ArtistCredit::default();
        let proposal = derive_edit(
            &subject,
            &outcome,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal
        .unwrap();
        assert_eq!(
            (proposal.target, proposal.approval.requires_confirmation()),
            (target("Cocaine", "Slowhand", Some("Various Artists")), true)
        );
    }

    #[test]
    fn moves_credit_the_album_to_its_main_artists_in_the_users_spelling() {
        let cases = [
            (
                "Luis Fonsi",
                "Despacito",
                credit(&[
                    ("luis-fonsi", "Luis Fonsi", " ft. "),
                    ("daddy-yankee", "Daddy Yankee", ""),
                ]),
                "Luis Fonsi",
            ),
            (
                "Wiz Khalifa",
                "See You Again",
                credit(&[
                    ("wiz-khalifa", "Wiz Khalifa", " feat. "),
                    ("charlie-puth", "Charlie Puth", ""),
                ]),
                "Wiz Khalifa",
            ),
            (
                "Simon & Garfunkel",
                "The Sound of Silence",
                credit(&[
                    ("paul-simon", "Simon", " & "),
                    ("art-garfunkel", "Garfunkel", ""),
                ]),
                "Simon & Garfunkel",
            ),
            (
                "JAY-Z",
                "Empire State of Mind",
                credit(&[
                    ("jay-z", "Jay\u{2010}Z", " & "),
                    ("alicia-keys", "Alicia Keys", ""),
                ]),
                "JAY-Z & Alicia Keys",
            ),
            (
                "Simon and Garfunkel",
                "Mrs. Robinson",
                credit(&[
                    ("paul-simon", "Simon", " & "),
                    ("art-garfunkel", "Garfunkel", ""),
                ]),
                "Simon and Garfunkel",
            ),
        ];
        for (artist, track, canonical_credit, expected) in cases {
            let subject = subject(
                artist,
                track,
                "Now That's What I Call Music!",
                Some("Various Artists"),
            );
            let now = group(
                "now",
                "Now That's What I Call Music!",
                PrimaryType::Album,
                &[SecondaryType::Compilation],
                "2017",
                credit(&[(VARIOUS, "Various Artists", "")]),
            );
            let canonical = Canonical {
                group: group(
                    track,
                    track,
                    PrimaryType::Single,
                    &[],
                    "2017",
                    canonical_credit,
                ),
                rung: Rung::Single,
                single_first: false,
            };
            let outcome = outcome(track, now, Some(canonical), Proof::SameRecording);
            let proposal = derive_edit(
                &subject,
                &outcome,
                Chosen::Deterministic,
                &[],
                &MovePolicy::default(),
                Naming::reviewer(),
            )
            .proposal
            .unwrap();
            assert_eq!(
                proposal.target,
                target(track, track, Some(expected)),
                "{track}"
            );
        }
    }

    #[test]
    fn move_policy_gates_each_kind_of_album_move() {
        let (cocaine, cocaine_outcome) = cocaine_on_cream();
        let holly = || credit(&[("buddy-holly", "Buddy Holly", "")]);
        let everyday = subject("Buddy Holly", "Everyday", "Peggy Sue", None);
        let everyday_outcome = outcome(
            "Everyday",
            group(
                "peggy-sue",
                "Peggy Sue",
                PrimaryType::Single,
                &[],
                "1957-09-20",
                holly(),
            ),
            plain_album(group(
                "buddy-holly",
                "Buddy Holly",
                PrimaryType::Album,
                &[],
                "1958-02-20",
                holly(),
            )),
            Proof::SameRecording,
        );
        let compilations = |mode| MovePolicy {
            compilations: mode,
            ..MovePolicy::default()
        };
        let singles = |mode| MovePolicy {
            singles: mode,
            ..MovePolicy::default()
        };
        let cases = [
            (
                &cocaine,
                &cocaine_outcome,
                compilations(MoveMode::Off),
                None,
            ),
            (
                &cocaine,
                &cocaine_outcome,
                compilations(MoveMode::Review),
                Some(true),
            ),
            (
                &cocaine,
                &cocaine_outcome,
                compilations(MoveMode::Auto),
                Some(false),
            ),
            (&everyday, &everyday_outcome, singles(MoveMode::Off), None),
            (
                &everyday,
                &everyday_outcome,
                singles(MoveMode::Auto),
                Some(true),
            ),
        ];
        for (subject, outcome, policy, expected) in cases {
            let actual = derive_edit(
                subject,
                outcome,
                Chosen::Deterministic,
                &[],
                &policy,
                Naming::reviewer(),
            )
            .proposal
            .map(|p| p.approval.requires_confirmation());
            assert_eq!(actual, expected, "{} with {policy:?}", subject.track);
        }
    }

    /// `None` means no proposal; otherwise the target and whether it needs review.
    #[test]
    fn proposals_and_their_approval() {
        let (cocaine, cocaine_outcome) = cocaine_on_cream();
        let holly = || credit(&[("buddy-holly", "Buddy Holly", "")]);
        let buddy_holly_album = group(
            "buddy-holly",
            "Buddy Holly",
            PrimaryType::Album,
            &[],
            "1958-02-20",
            holly(),
        );
        let peggy_sue_single = group(
            "peggy-sue",
            "Peggy Sue",
            PrimaryType::Single,
            &[],
            "1957-09-20",
            holly(),
        );
        let stones = || credit(&[("rolling-stones", "The Rolling Stones", "")]);
        let sticky_fingers = group(
            "4a4213d0-f224-3d51-bcca-476f61162681",
            "Sticky Fingers",
            PrimaryType::Album,
            &[],
            "1971-04-23",
            stones(),
        );
        let hot_rocks = group(
            "6e33402c-62db-3dc9-af86-2c1bef99ce2f",
            "Hot Rocks 1964–1971",
            PrimaryType::Album,
            &[SecondaryType::Compilation],
            "1971-12-20",
            stones(),
        );

        type Case = (
            &'static str,
            Subject,
            Outcome,
            Chosen,
            Vec<Target>,
            Option<(Target, bool)>,
        );
        let cases: Vec<Case> = vec![
            (
                "edition title within Slowhand's own release group",
                subject(
                    "Eric Clapton",
                    "Cocaine",
                    "Slowhand 35th Anniversary (Super Deluxe)",
                    None,
                ),
                outcome(
                    "Cocaine",
                    slowhand(),
                    plain_album(slowhand()),
                    Proof::SameRecording,
                ),
                Chosen::Deterministic,
                vec![],
                Some((target("Cocaine", "Slowhand", None), false)),
            ),
            (
                "live release group stays",
                subject(
                    "Wes Montgomery",
                    "Four on Six",
                    "Smokin' at the Half Note",
                    None,
                ),
                outcome(
                    "Four on Six",
                    smokin_at_the_half_note(),
                    plain_album(incredible_jazz_guitar()),
                    Proof::SameWork,
                ),
                Chosen::Deterministic,
                vec![],
                None,
            ),
            (
                "packaging-only title on a plain album",
                subject(
                    "The Rolling Stones",
                    "Wild Horses - 2009 Re-Mastered Digital Version",
                    "Sticky Fingers",
                    None,
                ),
                outcome(
                    "Wild Horses",
                    sticky_fingers.clone(),
                    plain_album(sticky_fingers.clone()),
                    Proof::SameRecording,
                ),
                Chosen::Deterministic,
                vec![],
                Some((
                    Target {
                        title: "Wild Horses".to_string(),
                        album: Some("Sticky Fingers".to_string()),
                        album_artist: None,
                    },
                    false,
                )),
            ),
            (
                "packaging stripped and moved off a compilation together",
                subject(
                    "The Rolling Stones",
                    "Wild Horses - 2009 Re-Mastered Digital Version",
                    "Hot Rocks 1964-1971",
                    None,
                ),
                outcome(
                    "Wild Horses",
                    hot_rocks,
                    plain_album(sticky_fingers),
                    Proof::SameRecording,
                ),
                Chosen::Deterministic,
                vec![],
                Some((
                    target("Wild Horses", "Sticky Fingers", Some("The Rolling Stones")),
                    false,
                )),
            ),
            (
                "single to album needs review",
                subject("Buddy Holly", "Everyday", "Peggy Sue", None),
                outcome(
                    "Everyday",
                    peggy_sue_single.clone(),
                    plain_album(buddy_holly_album.clone()),
                    Proof::SameRecording,
                ),
                Chosen::Deterministic,
                vec![],
                Some((target("Everyday", "Buddy Holly", Some("Buddy Holly")), true)),
            ),
            (
                "identity marker is never stripped deterministically",
                subject(
                    "Buddy Holly",
                    "Everyday - Single Version",
                    "Buddy Holly",
                    None,
                ),
                outcome(
                    "Everyday",
                    buddy_holly_album.clone(),
                    plain_album(buddy_holly_album.clone()),
                    Proof::SameRecording,
                ),
                Chosen::Deterministic,
                vec![],
                None,
            ),
            (
                "an adjudicated title change is proposed for review",
                subject(
                    "Buddy Holly",
                    "Everyday - Single Version",
                    "Buddy Holly",
                    None,
                ),
                outcome(
                    "Everyday",
                    buddy_holly_album.clone(),
                    plain_album(buddy_holly_album),
                    Proof::SameRecording,
                ),
                Chosen::Adjudicated,
                vec![],
                Some((
                    Target {
                        title: "Everyday".to_string(),
                        album: Some("Buddy Holly".to_string()),
                        album_artist: None,
                    },
                    true,
                )),
            ),
            (
                "adjudicated compilation move needs review",
                cocaine.clone(),
                cocaine_outcome.clone(),
                Chosen::Adjudicated,
                vec![],
                Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
            ),
            (
                "same work without an ISRC link needs review",
                cocaine.clone(),
                Outcome {
                    proof: Proof::SameWork,
                    ..cocaine_outcome.clone()
                },
                Chosen::Deterministic,
                vec![],
                Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
            ),
            (
                "ISRC duplicates more than 5 s apart need review",
                cocaine.clone(),
                Outcome {
                    proof: shared_isrc(5_001),
                    ..cocaine_outcome.clone()
                },
                Chosen::Deterministic,
                vec![],
                Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
            ),
            (
                "an album preceded by its single needs review",
                cocaine.clone(),
                Outcome {
                    canonical: Some(Canonical {
                        group: slowhand(),
                        rung: Rung::PlainAlbum,
                        single_first: true,
                    }),
                    ..cocaine_outcome.clone()
                },
                Chosen::Deterministic,
                vec![],
                Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
            ),
            (
                "an album whose first release may not carry the recording needs review",
                cocaine.clone(),
                Outcome {
                    canonical: Some(Canonical {
                        group: slowhand(),
                        rung: Rung::UndatedAlbum,
                        single_first: false,
                    }),
                    ..cocaine_outcome.clone()
                },
                Chosen::Deterministic,
                vec![],
                Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
            ),
            (
                "a researched outcome needs review even when settled",
                cocaine.clone(),
                Outcome {
                    proof: Proof::Researched {
                        by: "claude:opus".to_string(),
                    },
                    ..cocaine_outcome.clone()
                },
                Chosen::Deterministic,
                vec![],
                Some((target("Cocaine", "Slowhand", Some("Eric Clapton")), true)),
            ),
            (
                "a researched match never rewrites a title beyond packaging",
                subject(
                    "Eric Clapton",
                    "Cocaine (feat. J.J. Cale)",
                    "The Cream of Clapton",
                    None,
                ),
                Outcome {
                    proof: Proof::Researched {
                        by: "claude:opus".to_string(),
                    },
                    ..cocaine_outcome.clone()
                },
                Chosen::Adjudicated,
                vec![],
                Some((
                    target(
                        "Cocaine (feat. J.J. Cale)",
                        "Slowhand",
                        Some("Eric Clapton"),
                    ),
                    true,
                )),
            ),
            (
                "a rejected target is not proposed again",
                cocaine,
                cocaine_outcome,
                Chosen::Deterministic,
                vec![target("Cocaine", "Slowhand", Some("Eric Clapton"))],
                None,
            ),
        ];

        for (name, subject, outcome, chosen, rejected, expected) in cases {
            let actual = derive_edit(
                &subject,
                &outcome,
                chosen,
                &rejected,
                &MovePolicy::default(),
                Naming::reviewer(),
            )
            .proposal
            .map(|p| (p.target, p.approval.requires_confirmation()));
            assert_eq!(actual, expected, "{name}");
        }
    }
}
