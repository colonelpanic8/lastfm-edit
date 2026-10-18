//! Naming questions: whether an edition fix may also retitle the album. A release group's
//! title can differ from the user's by more than an edition suffix, as a restored
//! diacritic ("Béla Fleck and the Flecktones") or a MusicBrainz convention ("John
//! Lennon/Plastic Ono Band"). One answer per album covers every track on it.

use super::canonical::AlbumMove;
use super::outcome::{album_move, Outcome};
use super::title::{fold, strip_edition};
use uuid::Uuid;

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

#[cfg(test)]
mod tests {
    use super::*;

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
