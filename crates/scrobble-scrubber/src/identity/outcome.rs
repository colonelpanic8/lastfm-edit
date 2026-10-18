//! What a resolution concludes, and the pure derivation of edits from it. The approval
//! policy lives here and nowhere else.

use super::canonical::{album_move_allowed, AlbumMove, Canonical, Rung};
use super::mb::{RecordingId, ReleaseGroup};
use super::title::{classify_title_change, fold, parse_title, strip_edition, TitleChange};
use crate::rewrite::create_no_op_edit;
use crate::subject::Subject;
use lastfm_edit::ScrobbleEdit;

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
}

impl Proof {
    /// Strong enough to act on without a human.
    pub(crate) fn is_proven(&self) -> bool {
        match self {
            Proof::SameRecording => true,
            Proof::SharedIsrc { len_delta_ms, .. } => *len_delta_ms <= AUTO_ISRC_LEN_TOLERANCE_MS,
            Proof::SameWork | Proof::TitleOnly => false,
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

/// Pure: subject + outcome -> at most one proposal. `None` when nothing would change or
/// a human rejected the target for this subject.
///
/// Title and album keep the user's spelling when, with the packaging or edition suffix
/// removed, it folds equal to the MB text. Moves off a compilation, single or EP set the
/// album artist to the canonical credit (the executor otherwise keeps "Various Artists");
/// an edition fix leaves it alone.
pub(crate) fn derive_edit(
    subject: &Subject,
    outcome: &Outcome,
    chosen: Chosen,
    rejected: &[Target],
    policy: &MovePolicy,
) -> Option<Proposal> {
    let parsed = parse_title(&subject.track);
    let title_change = match classify_title_change(&parsed, &outcome.title) {
        // Only a pick among MusicBrainz options may rewrite a title beyond packaging; a
        // deterministic match may not drop "feat." credits or version notes.
        TitleChange::Other if chosen == Chosen::Deterministic => TitleChange::None,
        change => change,
    };
    let album_move =
        album_move(subject, outcome).filter(|(kind, _)| policy.mode(*kind) != MoveMode::Off);
    if title_change == TitleChange::None && album_move.is_none() {
        return None;
    }

    let title = match title_change {
        TitleChange::None => subject.track.clone(),
        TitleChange::PackagingOnly | TitleChange::Other => {
            in_users_spelling(&parsed.base, &outcome.title)
        }
    };
    let (album, album_artist) = match (album_move, &subject.album) {
        (Some((kind, canonical)), Some(scrobbled)) => (
            Some(in_users_spelling(
                &strip_edition(scrobbled),
                &canonical.group.title,
            )),
            match kind {
                AlbumMove::EditionToGroup => subject.album_artist.clone(),
                AlbumMove::FromCompilation | AlbumMove::FromSingleOrEp => {
                    credited_album_artist(canonical).or_else(|| subject.album_artist.clone())
                }
            },
        ),
        _ => (subject.album.clone(), subject.album_artist.clone()),
    };
    let target = Target {
        title,
        album,
        album_artist,
    };
    if rejected.contains(&target) {
        return None;
    }

    let mut edit = create_no_op_edit(&subject.representative_track(0, None));
    edit.track_name = Some(target.title.clone());
    edit.album_name = target.album.clone();
    edit.album_artist_name = target.album_artist.clone();

    Some(Proposal {
        edit,
        approval: approval(chosen, &outcome.proof, title_change, album_move, policy),
        rationale: rationale(subject, outcome, chosen, title_change, album_move),
        target,
    })
}

/// The user's own text when it already says what MusicBrainz says, so typography
/// ("Who's" vs "Who\u{2019}s") never splits their scrobbles.
fn in_users_spelling(users: &str, mb: &str) -> String {
    if fold(users) == fold(mb) { users } else { mb }.to_string()
}

fn credited_album_artist(canonical: &Canonical) -> Option<String> {
    Some(canonical.group.credit.display()).filter(|credit| !credit.trim().is_empty())
}

fn album_move<'a>(subject: &Subject, outcome: &'a Outcome) -> Option<(AlbumMove, &'a Canonical)> {
    let (Some(album), Some(scrobbled), Some(canonical)) =
        (&subject.album, &outcome.scrobbled_group, &outcome.canonical)
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
    album: Option<(AlbumMove, &Canonical)>,
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
    if let Some((kind, canonical)) = album {
        let from = outcome
            .scrobbled_group
            .as_ref()
            .map_or_else(String::new, ReleaseGroup::describe);
        let why = match kind {
            AlbumMove::EditionToGroup => "edition title of the same release group",
            AlbumMove::FromCompilation => "off a compilation",
            AlbumMove::FromSingleOrEp => "off a single or EP",
        };
        lines.push(format!(
            "Album: {from} -> {} ({why})",
            canonical.group.describe()
        ));
    }
    lines.push(match &outcome.proof {
        Proof::SameRecording => "Proof: the same MusicBrainz recording".to_string(),
        Proof::SharedIsrc { isrc, len_delta_ms } => format!(
            "Proof: recordings share ISRC {isrc}, lengths {:.1} s apart",
            f64::from(*len_delta_ms) / 1000.0
        ),
        Proof::SameWork => "Proof: same work, different recording".to_string(),
        Proof::TitleOnly => "Proof: title and artist match only".to_string(),
    });
    if chosen == Chosen::Adjudicated {
        lines.push("Chosen among several MusicBrainz options".to_string());
    }
    lines.join("\n")
}

pub(crate) fn approval(
    chosen: Chosen,
    proof: &Proof,
    title: TitleChange,
    album: Option<(AlbumMove, &Canonical)>,
    policy: &MovePolicy,
) -> Approval {
    let proven = proof.is_proven();
    let album_safe = album.is_none_or(|(kind, canonical)| {
        let kind_safe = match kind {
            AlbumMove::EditionToGroup => true,
            AlbumMove::FromCompilation => credited_album_artist(canonical).is_some(),
            AlbumMove::FromSingleOrEp => false,
        };
        kind_safe
            && policy.mode(kind) == MoveMode::Auto
            && canonical.rung == Rung::PlainAlbum
            && !canonical.single_first
    });
    let title_safe = matches!(title, TitleChange::None | TitleChange::PackagingOnly);
    if chosen == Chosen::Deterministic && proven && album_safe && title_safe {
        Approval::Auto(AutoProof(()))
    } else {
        Approval::Review
    }
}

#[cfg(test)]
mod tests {
    use super::super::canonical::examples::*;
    use super::super::mb::{ArtistCredit, PrimaryType, SecondaryType};
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
        )
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
        let mingus_ah_um = group(
            "mingus-ah-um",
            "Mingus Ah Um",
            PrimaryType::Album,
            &[],
            "1959-10",
            credit(&[("charles-mingus", "Charles Mingus", "")]),
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
            (
                subject(
                    "Charles Mingus",
                    "Better Git It in Your Soul",
                    "AH UM - 50th Anniversary (Legacy Edition)",
                    None,
                ),
                "Better Git It in Your Soul",
                mingus_ah_um,
                target("Better Git It in Your Soul", "Mingus Ah Um", None),
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
            )
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
    fn compilation_move_without_a_canonical_credit_keeps_the_album_artist_for_review() {
        let (subject, mut outcome) = cocaine_on_cream();
        outcome.canonical.as_mut().unwrap().group.credit = ArtistCredit::default();
        let proposal = derive_edit(
            &subject,
            &outcome,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
        )
        .unwrap();
        assert_eq!(
            (proposal.target, proposal.approval.requires_confirmation()),
            (target("Cocaine", "Slowhand", Some("Various Artists")), true)
        );
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
            let actual = derive_edit(subject, outcome, Chosen::Deterministic, &[], &policy)
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
            )
            .map(|p| (p.target, p.approval.requires_confirmation()));
            assert_eq!(actual, expected, "{name}");
        }
    }
}
