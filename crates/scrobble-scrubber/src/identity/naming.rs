//! Naming questions: whether a fix may also retitle the album or the track. A release
//! group's title can differ from the user's by more than an edition suffix, as a restored
//! diacritic ("Béla Fleck and the Flecktones") or a MusicBrainz convention ("John
//! Lennon/Plastic Ono Band"); one answer per album covers every track on it. A recording
//! picked among options can carry MusicBrainz's own annotations ("Who Loves the Sun (2012
//! remaster)") or lack the user's version marker ("Mack the Knife" for "Mack the Knife
//! [Live]"); one answer per track title covers every album it was played on.

use super::adjudicate::{truncate_bytes, AdjudicateError, MAX_RAW_BYTES, MAX_REASON_CHARS};
use super::book::Snapshot;
use super::canonical::AlbumMove;
use super::llm::{ChoiceRequest, LlmBackend};
use super::mb::ReleaseGroup;
use super::outcome::{
    album_move, derive_first, title_change, Chosen, MovePolicy, Naming, Outcome, RetitleJudge,
};
use super::title::{fold, strip_edition, TitleChange};
use crate::subject::{ScrobbleKey, Subject};
use std::collections::BTreeMap;
use std::fmt::Write;
use uuid::Uuid;

/// Bump when the prompt or case rendering changes; recorded with every LLM verdict.
pub(crate) const NAMING_PROMPT_VERSION: u32 = 1;

pub(crate) const NAMING_PROMPT: &str = "\
You help keep a music listening history (last.fm scrobbles) tidy. The user's title for an album \
and the MusicBrainz release group's title for the same album differ by more than an edition note \
such as \"(Deluxe Edition)\". Decide which title the user's scrobbles should carry.

Prefer the title as the album was officially released and as last.fm and streaming services \
commonly show it. Choose:
- \"musicbrainz\" when the MusicBrainz title is that title and the user's is a variant of it, such as \
missing diacritics or different capitalization (\"Bela Fleck\" for \"Béla Fleck\");
- \"keep\" when the user's title is that title and the MusicBrainz title follows a \
MusicBrainz-specific convention (such as \"Artist/Title\") or a wording less commonly used. When \
both titles are equally valid, choose \"keep\";
- \"unsure\" when you cannot tell. A person reviews unsure answers.

Reply with JSON only: {\"choice\": \"keep\" | \"musicbrainz\" | \"unsure\", \"confidence\": \
\"high\" | \"medium\" | \"low\", \"reason\": \"<one sentence>\"}";

pub(crate) const TRACK_NAMING_PROMPT: &str = "\
You help keep a music listening history (last.fm scrobbles) tidy. A scrobbled track was matched \
to a MusicBrainz recording whose title differs from the user's title by more than packaging such \
as \"- Remastered 2009\". Decide which title the user's scrobbles should carry.

Prefer the title as the track was officially released and as last.fm and streaming services \
show it. MusicBrainz recording titles can carry text that belongs to MusicBrainz rather than to \
any release: disambiguation, and session, venue, date or remaster notes (\"(live at the Fillmore \
East, March 7, 1970)\", \"(2012 remaster)\"). Never adopt such text. The user's title can carry \
a version marker (live, remix, demo, acoustic) that tells a genuinely different recording apart \
from the studio original; never drop one unless the MusicBrainz title says the same thing. \
Choose:
- \"musicbrainz\" when the MusicBrainz title is the released title and the user's is a variant of \
it, such as a misspelling, missing diacritics or different capitalization, and adopting it adds \
no annotation and drops no version marker;
- \"keep\" when the MusicBrainz title adds an annotation as above, drops a version marker the \
user's title has, or is otherwise no better than the user's. When both titles are equally valid, \
choose \"keep\";
- \"unsure\" when you cannot tell. A person reviews unsure answers.

Reply with JSON only: {\"choice\": \"keep\" | \"musicbrainz\" | \"unsure\", \"confidence\": \
\"high\" | \"medium\" | \"low\", \"reason\": \"<one sentence>\"}";

const MAX_TRACKS: usize = 8;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum NamingKind {
    #[default]
    Album,
    Track,
}

/// One album's or track title's question: the user's title against MusicBrainz's.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct NamingQuestion {
    /// The default and the `album` alias read verdicts logged before track questions.
    #[serde(default)]
    pub kind: NamingKind,
    pub artist: String,
    /// The user's album title without its edition suffix, or the track title as scrobbled.
    #[serde(alias = "album")]
    pub users_title: String,
    /// The MusicBrainz release group's or recording's title.
    pub title: String,
}

impl NamingQuestion {
    pub fn album(artist: &str, users_title: &str, title: &str) -> Self {
        Self::new(NamingKind::Album, artist, users_title, title)
    }

    pub fn track(artist: &str, users_title: &str, title: &str) -> Self {
        Self::new(NamingKind::Track, artist, users_title, title)
    }

    fn new(kind: NamingKind, artist: &str, users_title: &str, title: &str) -> Self {
        Self {
            kind,
            artist: artist.to_string(),
            users_title: users_title.to_string(),
            title: title.to_string(),
        }
    }
}

/// What an edition fix does to the album title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EditionFix {
    /// Only an edition suffix goes; the rest folds equal to the group title.
    SuffixOnly,
    /// The title itself changes, diacritics and wording included.
    Retitle(NamingQuestion),
}

pub(crate) fn classify_edition_fix(artist: &str, album: &str, group_title: &str) -> EditionFix {
    let base = strip_edition(album);
    if fold(&base) == fold(group_title) {
        return EditionFix::SuffixOnly;
    }
    EditionFix::Retitle(NamingQuestion::album(artist, &base, group_title))
}

/// The questions `key` raises under `outcome` whatever the move policy: its album's, if
/// the edition fix retitles it, and its track's, if the title changes beyond packaging.
pub(crate) fn naming_questions(
    key: &ScrobbleKey,
    outcome: &Outcome,
    chosen: Chosen,
) -> Vec<NamingQuestion> {
    let album = key.album.as_deref();
    let album_question = match album_move(album, outcome) {
        Some((AlbumMove::EditionToGroup, canonical)) => {
            match classify_edition_fix(
                &key.artist,
                album.unwrap_or_default(),
                &canonical.group.title,
            ) {
                EditionFix::SuffixOnly => None,
                EditionFix::Retitle(question) => Some(question),
            }
        }
        _ => None,
    };
    let track_question = (title_change(&key.track, outcome, chosen) == TitleChange::Other)
        .then(|| NamingQuestion::track(&key.artist, &key.track, &outcome.title));
    album_question.into_iter().chain(track_question).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamingVerdict {
    /// The user's title stays; an edition suffix still goes.
    Keep,
    /// The release group's title replaces the user's.
    #[serde(rename = "musicbrainz")]
    MusicBrainz,
    /// A person decides.
    Unsure,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NamingJudge {
    /// `model` is the backend label.
    Llm { model: String, prompt_version: u32 },
    /// A person rejected identity intent `intent`, which made the retitle.
    Human { intent: Uuid },
}

impl NamingJudge {
    pub fn describe(&self) -> String {
        match self {
            NamingJudge::Llm { model, .. } => model.clone(),
            NamingJudge::Human { .. } => "a rejection in review".to_string(),
        }
    }
}

/// The latest answer to a question, folded from the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamingJudgement {
    pub verdict: NamingVerdict,
    pub reason: String,
    pub by: NamingJudge,
    /// Unix seconds.
    pub at: u64,
}

/// Everything the model sees about one question.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NamingCase {
    pub question: NamingQuestion,
    /// The canonical release group for an album; the scrobbled one, if any, for a track.
    pub group: Option<ReleaseGroup>,
    /// The user's album strings that raise the question.
    pub albums: Vec<String>,
    /// Tracks the user played on the album, most-played first; none for a track.
    pub tracks: Vec<String>,
    pub plays: u32,
}

pub(crate) struct NamingAnswer {
    pub verdict: NamingVerdict,
    pub reason: String,
    /// The backend label.
    pub by: String,
    /// Raw model text, truncated to 4 KiB.
    pub raw: String,
}

/// Asks the model to pick "keep" or "musicbrainz"; anything else counts as unsure.
pub(crate) async fn judge(
    backend: &dyn LlmBackend,
    case: &NamingCase,
) -> Result<NamingAnswer, AdjudicateError> {
    let raw = backend
        .choose(ChoiceRequest {
            system: match case.question.kind {
                NamingKind::Album => NAMING_PROMPT,
                NamingKind::Track => TRACK_NAMING_PROMPT,
            }
            .to_string(),
            user: render_case(case),
            allowed: ["keep", "musicbrainz", "unsure"].map(String::from).to_vec(),
        })
        .await?;
    let answer = raw.choice.trim();
    let verdict = if answer.eq_ignore_ascii_case("keep") {
        NamingVerdict::Keep
    } else if answer.eq_ignore_ascii_case("musicbrainz") {
        NamingVerdict::MusicBrainz
    } else {
        NamingVerdict::Unsure
    };
    Ok(NamingAnswer {
        verdict,
        reason: raw.reason.trim().chars().take(MAX_REASON_CHARS).collect(),
        by: backend.label(),
        raw: truncate_bytes(&raw.text, MAX_RAW_BYTES).to_string(),
    })
}

fn render_case(case: &NamingCase) -> String {
    let quoted = |values: &[String]| {
        values
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let question = &case.question;
    let (heading, albums) = match question.kind {
        NamingKind::Album => ("Album", "the user's album strings"),
        NamingKind::Track => ("Track", "albums the user played it on"),
    };
    let mut out = String::new();
    let _ = writeln!(out, "{heading}");
    let _ = writeln!(out, "  artist: \"{}\"", question.artist);
    let _ = writeln!(out, "  the user's title: \"{}\"", question.users_title);
    let _ = writeln!(out, "  MusicBrainz title: \"{}\"", question.title);
    if let Some(group) = &case.group {
        let _ = writeln!(
            out,
            "  MusicBrainz release group: {}, credited to \"{}\"",
            group.describe(),
            group.credit.display()
        );
    }
    let _ = writeln!(out, "  {albums}: {}", quoted(&case.albums));
    if question.kind == NamingKind::Album {
        let _ = writeln!(out, "  tracks played: {}", quoted(&case.tracks));
    }
    let _ = writeln!(out, "  plays: {}", case.plays);
    let _ = write!(
        out,
        "\nAnswer keep to use \"{}\", musicbrainz to use \"{}\", or unsure.",
        question.users_title, question.title
    );
    out
}

/// One case per question derivation waits on for `plays`' keys under `policy`, with the
/// most-played key that raises it; most-played first. Only these answers can change an
/// edit.
pub(crate) fn naming_cases<'a>(
    plays: impl IntoIterator<Item = (&'a ScrobbleKey, u32)>,
    snapshot: &Snapshot,
    policy: &MovePolicy,
) -> Vec<(ScrobbleKey, NamingCase)> {
    struct Asked<'a> {
        group: Option<&'a ReleaseGroup>,
        keys: Vec<(u32, &'a ScrobbleKey)>,
    }
    let naming = Naming {
        verdicts: &snapshot.naming,
        judge: RetitleJudge::Llm,
    };
    let mut asked: BTreeMap<NamingQuestion, Asked> = BTreeMap::new();
    for (key, key_plays) in plays {
        let Some(entry) = snapshot.keys.get(key) else {
            continue;
        };
        let subject = Subject {
            artist: key.artist.clone(),
            track: key.track.clone(),
            album: key.album.clone(),
            album_artist: None,
        };
        let Some((outcome, derived)) =
            derive_first(&subject, entry.plannable(), &entry.rejected, policy, naming)
        else {
            continue;
        };
        let canonical = outcome.canonical.as_ref().map(|canonical| &canonical.group);
        for question in derived.waiting {
            let group = match question.kind {
                NamingKind::Album => canonical,
                NamingKind::Track => outcome.scrobbled_group.as_ref().or(canonical),
            };
            asked
                .entry(question)
                .or_insert_with(|| Asked {
                    group,
                    keys: Vec::new(),
                })
                .keys
                .push((key_plays, key));
        }
    }
    let mut cases: Vec<(ScrobbleKey, NamingCase)> = asked
        .into_iter()
        .map(|(question, mut asked)| {
            asked
                .keys
                .sort_by(|(a_plays, a), (b_plays, b)| b_plays.cmp(a_plays).then(a.cmp(b)));
            let mut albums: Vec<String> = Vec::new();
            let mut tracks: Vec<String> = Vec::new();
            for (_, key) in &asked.keys {
                let scrobbled = key.album.clone().unwrap_or_default();
                if !albums.contains(&scrobbled) {
                    albums.push(scrobbled);
                }
                if question.kind == NamingKind::Album
                    && !tracks.contains(&key.track)
                    && tracks.len() < MAX_TRACKS
                {
                    tracks.push(key.track.clone());
                }
            }
            let case = NamingCase {
                question,
                group: asked.group.cloned(),
                albums,
                tracks,
                plays: asked.keys.iter().map(|(plays, _)| plays).sum(),
            };
            (asked.keys[0].1.clone(), case)
        })
        .collect();
    cases.sort_by(|(_, a), (_, b)| b.plays.cmp(&a.plays).then(a.question.cmp(&b.question)));
    cases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::llm::scripted::ScriptedBackend;
    use crate::identity::llm::RawChoice;
    use crate::identity::testing::{credit, group};

    #[tokio::test]
    async fn answers_outside_keep_and_musicbrainz_are_unsure() {
        let case = NamingCase {
            question: NamingQuestion::album(
                "John Lennon",
                "Plastic Ono Band",
                "John Lennon/Plastic Ono Band",
            ),
            group: Some(group(
                "8be83468-4917-390d-82b6-d890f058cd22",
                "John Lennon/Plastic Ono Band",
                super::super::mb::PrimaryType::Album,
                &[],
                1970,
                credit(&[("4d5447d7", "John Lennon", "")]),
            )),
            albums: vec!["Plastic Ono Band".into()],
            tracks: vec!["Mother".into()],
            plays: 5,
        };
        let cases = [
            ("MusicBrainz", NamingVerdict::MusicBrainz),
            (" keep ", NamingVerdict::Keep),
            ("unsure", NamingVerdict::Unsure),
            ("John Lennon/Plastic Ono Band", NamingVerdict::Unsure),
            ("", NamingVerdict::Unsure),
        ];
        for (choice, expected) in cases {
            let backend = ScriptedBackend::default();
            backend.answers.lock().unwrap().push(RawChoice {
                choice: choice.into(),
                reason: format!("  {}  ", "é".repeat(400)),
                ..RawChoice::default()
            });
            let answer = judge(&backend, &case).await.unwrap();
            assert_eq!(
                (answer.verdict, answer.reason, answer.by),
                (expected, "é".repeat(300), "scripted".to_string()),
                "{choice:?}"
            );
        }
    }

    #[test]
    fn edition_fixes_that_change_more_than_the_suffix_are_retitles() {
        let question = |artist: &str, album: &str, title: &str| {
            EditionFix::Retitle(NamingQuestion::album(artist, album, title))
        };
        let cases = [
            (
                "Béla Fleck and the Flecktones",
                "Bela Fleck And The Flecktones",
                "Béla Fleck and the Flecktones",
                question(
                    "Béla Fleck and the Flecktones",
                    "Bela Fleck And The Flecktones",
                    "Béla Fleck and the Flecktones",
                ),
            ),
            (
                "John Lennon",
                "Plastic Ono Band (Remastered)",
                "John Lennon/Plastic Ono Band",
                question(
                    "John Lennon",
                    "Plastic Ono Band",
                    "John Lennon/Plastic Ono Band",
                ),
            ),
            (
                "Jeff Buckley",
                "Grace (Legacy Edition)",
                "Grace",
                EditionFix::SuffixOnly,
            ),
            (
                "Eric Clapton",
                "Slowhand 35th Anniversary (Super Deluxe)",
                "Slowhand",
                EditionFix::SuffixOnly,
            ),
            (
                "The Who",
                "Who's Next (Deluxe Edition)",
                "Who\u{2019}s Next",
                EditionFix::SuffixOnly,
            ),
        ];
        for (artist, album, title, expected) in cases {
            assert_eq!(
                classify_edition_fix(artist, album, title),
                expected,
                "{album}"
            );
        }
    }
}
