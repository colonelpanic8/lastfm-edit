//! Naming questions: whether an edition fix may also retitle the album. A release group's
//! title can differ from the user's by more than an edition suffix, as a restored
//! diacritic ("Béla Fleck and the Flecktones") or a MusicBrainz convention ("John
//! Lennon/Plastic Ono Band"). One answer per album covers every track on it.

use super::adjudicate::{truncate_bytes, AdjudicateError, MAX_RAW_BYTES, MAX_REASON_CHARS};
use super::book::Snapshot;
use super::canonical::AlbumMove;
use super::llm::{ChoiceRequest, LlmBackend};
use super::mb::ReleaseGroup;
use super::outcome::{album_move, Chosen, Outcome};
use super::title::{fold, strip_edition};
use crate::subject::ScrobbleKey;
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

const MAX_TRACKS: usize = 8;

/// One album's question: the user's title against the release group's.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct NamingQuestion {
    pub artist: String,
    /// The user's album title without its edition suffix.
    pub album: String,
    /// The MusicBrainz release group's title.
    pub title: String,
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
    EditionFix::Retitle(NamingQuestion {
        artist: artist.to_string(),
        album: base,
        title: group_title.to_string(),
    })
}

/// The question a scrobble on `album` raises under `outcome`, if its album fix retitles.
pub(crate) fn naming_question(
    artist: &str,
    album: Option<&str>,
    outcome: &Outcome,
) -> Option<NamingQuestion> {
    let (AlbumMove::EditionToGroup, canonical) = album_move(album, outcome)? else {
        return None;
    };
    match classify_edition_fix(artist, album?, &canonical.group.title) {
        EditionFix::SuffixOnly => None,
        EditionFix::Retitle(question) => Some(question),
    }
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
    /// A person rejected identity intent `intent`, which retitled the album.
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
    pub group: ReleaseGroup,
    /// The user's album strings that raise the question.
    pub albums: Vec<String>,
    /// Tracks the user played on it, most-played first.
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
            system: NAMING_PROMPT.to_string(),
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
    let mut out = String::new();
    let _ = writeln!(out, "Album");
    let _ = writeln!(out, "  artist: \"{}\"", question.artist);
    let _ = writeln!(out, "  the user's title: \"{}\"", question.album);
    let _ = writeln!(out, "  MusicBrainz title: \"{}\"", question.title);
    let _ = writeln!(
        out,
        "  MusicBrainz release group: {}, credited to \"{}\"",
        case.group.describe(),
        case.group.credit.display()
    );
    let _ = writeln!(out, "  the user's album strings: {}", quoted(&case.albums));
    let _ = writeln!(out, "  tracks played: {}", quoted(&case.tracks));
    let _ = writeln!(out, "  plays: {}", case.plays);
    let _ = write!(
        out,
        "\nAnswer keep to use \"{}\", musicbrainz to use \"{}\", or unsure.",
        question.album, question.title
    );
    out
}

/// One case per unanswered question the deterministic outcomes of `plays`' keys raise,
/// with the most-played key that raises it; most-played first.
pub(crate) fn naming_cases<'a>(
    plays: impl IntoIterator<Item = (&'a ScrobbleKey, u32)>,
    snapshot: &Snapshot,
) -> Vec<(ScrobbleKey, NamingCase)> {
    struct Album<'a> {
        group: &'a ReleaseGroup,
        keys: Vec<(u32, &'a ScrobbleKey)>,
    }
    let mut albums: BTreeMap<NamingQuestion, Album> = BTreeMap::new();
    for (key, key_plays) in plays {
        let Some(entry) = snapshot.keys.get(key) else {
            continue;
        };
        for (outcome, chosen) in entry.plannable() {
            let (Chosen::Deterministic, Some(canonical)) = (chosen, &outcome.canonical) else {
                continue;
            };
            let Some(question) = naming_question(&key.artist, key.album.as_deref(), outcome) else {
                continue;
            };
            if snapshot.naming.contains_key(&question) {
                continue;
            }
            albums
                .entry(question)
                .or_insert_with(|| Album {
                    group: &canonical.group,
                    keys: Vec::new(),
                })
                .keys
                .push((key_plays, key));
        }
    }
    let mut cases: Vec<(ScrobbleKey, NamingCase)> = albums
        .into_iter()
        .map(|(question, mut album)| {
            album
                .keys
                .sort_by(|(a_plays, a), (b_plays, b)| b_plays.cmp(a_plays).then(a.cmp(b)));
            let mut albums: Vec<String> = Vec::new();
            let mut tracks: Vec<String> = Vec::new();
            for (_, key) in &album.keys {
                let scrobbled = key.album.clone().unwrap_or_default();
                if !albums.contains(&scrobbled) {
                    albums.push(scrobbled);
                }
                if !tracks.contains(&key.track) && tracks.len() < MAX_TRACKS {
                    tracks.push(key.track.clone());
                }
            }
            let case = NamingCase {
                question,
                group: album.group.clone(),
                albums,
                tracks,
                plays: album.keys.iter().map(|(plays, _)| plays).sum(),
            };
            (album.keys[0].1.clone(), case)
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
            question: NamingQuestion {
                artist: "John Lennon".into(),
                album: "Plastic Ono Band".into(),
                title: "John Lennon/Plastic Ono Band".into(),
            },
            group: group(
                "8be83468-4917-390d-82b6-d890f058cd22",
                "John Lennon/Plastic Ono Band",
                super::super::mb::PrimaryType::Album,
                &[],
                1970,
                credit(&[("4d5447d7", "John Lennon", "")]),
            ),
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
            EditionFix::Retitle(NamingQuestion {
                artist: artist.into(),
                album: album.into(),
                title: title.into(),
            })
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
