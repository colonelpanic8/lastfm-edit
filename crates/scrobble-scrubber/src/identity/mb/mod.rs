//! MusicBrainz access: domain types parsed at the boundary, the [`MbLookup`] seam the
//! resolver depends on, and the paced + cached HTTP implementation.

mod cache;
mod client;

pub(crate) use client::MusicBrainz;

use std::collections::BTreeSet;
use std::time::Duration;

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct RecordingId(pub String);

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct ReleaseGroupId(pub String);

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct ArtistId(pub String);

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct WorkId(pub String);

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Isrc(pub String);

/// "1977", "1977-11", "1977-11-25". Derived `Ord` sorts a bare year before any dated
/// day in that year, which is what "earliest" wants.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PartialDate {
    pub year: u16,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

impl PartialDate {
    /// Parses MB's date strings; `None` for empty or malformed input.
    pub fn parse(s: &str) -> Option<Self> {
        fn field<T: std::str::FromStr + PartialOrd>(
            s: &str,
            len: usize,
            range: std::ops::RangeInclusive<T>,
        ) -> Option<T> {
            if s.len() != len || !s.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            s.parse().ok().filter(|v| range.contains(v))
        }

        let mut fields = s.trim().split('-');
        let year = field(fields.next()?, 4, 1..=9999)?;
        let month = match fields.next() {
            Some(m) => Some(field(m, 2, 1..=12)?),
            None => None,
        };
        let day = match fields.next() {
            Some(d) => Some(field(d, 2, 1..=31)?),
            None => None,
        };
        if fields.next().is_some() {
            return None;
        }
        Some(Self { year, month, day })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CreditPart {
    pub artist: ArtistId,
    /// The name as credited on this entity (may differ from the artist's canonical name).
    pub name: String,
    pub joinphrase: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArtistCredit {
    pub parts: Vec<CreditPart>,
}

impl ArtistCredit {
    /// Names joined with their joinphrases: "Derek and the Dominos", "Simon & Garfunkel".
    pub fn display(&self) -> String {
        self.parts
            .iter()
            .map(|part| format!("{}{}", part.name, part.joinphrase))
            .collect()
    }

    pub fn artist_ids(&self) -> BTreeSet<&ArtistId> {
        self.parts.iter().map(|part| &part.artist).collect()
    }

    pub fn is_various_artists(&self) -> bool {
        self.parts
            .iter()
            .any(|part| part.artist.0 == VARIOUS_ARTISTS_ID)
    }
}

const VARIOUS_ARTISTS_ID: &str = "89ad4ac3-39f7-470e-963a-56509c546377";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PrimaryType {
    Album,
    Single,
    Ep,
    Broadcast,
    Other,
}

impl PrimaryType {
    fn name(self) -> &'static str {
        match self {
            Self::Album => "Album",
            Self::Single => "Single",
            Self::Ep => "EP",
            Self::Broadcast => "Broadcast",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SecondaryType {
    Compilation,
    Live,
    Soundtrack,
    Remix,
    DjMix,
    Mixtape,
    Demo,
    Spokenword,
    Interview,
    Audiobook,
    Other,
}

impl SecondaryType {
    fn name(self) -> &'static str {
        match self {
            Self::Compilation => "Compilation",
            Self::Live => "Live",
            Self::Soundtrack => "Soundtrack",
            Self::Remix => "Remix",
            Self::DjMix => "DJ-mix",
            Self::Mixtape => "Mixtape/Street",
            Self::Demo => "Demo",
            Self::Spokenword => "Spokenword",
            Self::Interview => "Interview",
            Self::Audiobook => "Audiobook",
            Self::Other => "Other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReleaseGroup {
    pub id: ReleaseGroupId,
    pub title: String,
    pub primary: Option<PrimaryType>,
    pub secondary: Vec<SecondaryType>,
    pub first_release: Option<PartialDate>,
    pub credit: ArtistCredit,
}

impl ReleaseGroup {
    /// "Slowhand (Album, 1977)" / "The Cream of Clapton (Album + Compilation, 1995)".
    pub fn describe(&self) -> String {
        let types: Vec<&str> = self
            .primary
            .map(PrimaryType::name)
            .into_iter()
            .chain(self.secondary.iter().map(|t| t.name()))
            .collect();
        let details: Vec<String> = [
            (!types.is_empty()).then(|| types.join(" + ")),
            self.first_release.map(|date| date.year.to_string()),
        ]
        .into_iter()
        .flatten()
        .collect();
        if details.is_empty() {
            self.title.clone()
        } else {
            format!("{} ({})", self.title, details.join(", "))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReleaseRef {
    pub title: String,
    pub official: bool,
    pub date: Option<PartialDate>,
    pub group: ReleaseGroup,
}

/// A recording search hit. `snippet_releases` is PARTIAL: good for "is it on album X"
/// and that release's group types, never for dates or canonical choice.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecordingHit {
    pub id: RecordingId,
    pub score: u8,
    pub title: String,
    pub disambiguation: String,
    pub length_ms: Option<u32>,
    pub credit: ArtistCredit,
    pub snippet_releases: Vec<ReleaseRef>,
}

/// One work a recording performs, with the relationship attributes ("live", "cover",
/// "medley", "partial", ...).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkLink {
    pub work: WorkId,
    pub attributes: Vec<String>,
}

/// Recording lookup with `inc=isrcs+work-rels+artist-credits`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecordingDetail {
    /// Differs from the requested id when MB merged it (redirect followed).
    pub id: RecordingId,
    pub title: String,
    pub disambiguation: String,
    pub length_ms: Option<u32>,
    pub credit: ArtistCredit,
    pub isrcs: Vec<Isrc>,
    pub works: Vec<WorkLink>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordingQuery {
    pub title: String,
    pub artist: String,
    pub release: Option<String>,
}

impl RecordingQuery {
    /// The Lucene query string (quoted and escaped). Also the cache key.
    pub fn lucene(&self) -> String {
        fn phrase(field: &str, value: &str) -> String {
            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            format!("{field}:\"{escaped}\"")
        }

        let mut query = format!(
            "{} AND {}",
            phrase("recording", &self.title),
            phrase("artist", &self.artist)
        );
        if let Some(release) = &self.release {
            query.push_str(" AND ");
            query.push_str(&phrase("release", release));
        }
        query
    }
}

/// A list that stopped at a cap or page limit before reaching MB's reported count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Truncated(pub bool);

/// Everything the resolver needs from MusicBrainz. Implementations are cache-first.
/// The real one is [`MusicBrainz`]; tests use `identity::testing::FixtureMb`.
#[async_trait::async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async-trait adds must_use to generated futures"
)]
pub trait MbLookup: Send + Sync {
    /// One page of up to 100 hits, best first; `Truncated` when MB matched more.
    async fn search_recordings(
        &self,
        query: &RecordingQuery,
    ) -> Result<(Vec<RecordingHit>, Truncated), MbError>;
    async fn recording(&self, id: &RecordingId) -> Result<RecordingDetail, MbError>;
    /// Recordings sharing an ISRC (`/isrc/<code>`).
    async fn isrc_members(&self, isrc: &Isrc) -> Result<Vec<RecordingId>, MbError>;
    /// Recordings of a work (`recording?work=<id>`), capped.
    async fn work_recordings(
        &self,
        work: &WorkId,
        cap: usize,
    ) -> Result<(Vec<RecordingId>, Truncated), MbError>;
    /// Official releases containing a recording, via paged
    /// `release?recording=<id>&status=official&inc=release-groups+artist-credits`.
    async fn recording_releases(
        &self,
        id: &RecordingId,
        cap: usize,
    ) -> Result<(Vec<ReleaseRef>, Truncated), MbError>;
}

#[derive(Debug, thiserror::Error)]
pub enum MbError {
    #[error("rate limited; retry after {0:?}")]
    RateLimited(Duration),
    #[error("transport: {0}")]
    Transport(String),
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("cache: {0}")]
    Cache(String),
    #[error("not found")]
    NotFound,
}

/// MusicBrainz rejects anonymous clients. Constructible only with contact info.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAgent(String);

impl UserAgent {
    /// `app` like "scrobble-scrubber/0.2", `contact` an email or URL.
    pub fn new(app: &str, contact: &str) -> Result<Self, MbError> {
        let (app, contact) = (app.trim(), contact.trim());
        if app.is_empty() || contact.is_empty() {
            return Err(MbError::Parse(
                "a MusicBrainz user agent needs an app name and contact info".to_string(),
            ));
        }
        Ok(Self(format!("{app} ( {contact} )")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u16, month: Option<u8>, day: Option<u8>) -> Option<PartialDate> {
        Some(PartialDate { year, month, day })
    }

    #[test]
    fn parses_musicbrainz_dates() {
        let cases = [
            ("1977", date(1977, None, None)),
            ("1977-11", date(1977, Some(11), None)),
            ("1977-11-25", date(1977, Some(11), Some(25))),
            ("", None),
            ("????", None),
            ("1977-13", None),
            ("1977-11-25-01", None),
            ("77", None),
        ];
        for (input, expected) in cases {
            assert_eq!(PartialDate::parse(input), expected, "{input}");
        }
        assert!(PartialDate::parse("1977") < PartialDate::parse("1977-01-01"));
    }

    #[test]
    fn credits_read_as_credited() {
        let credit = ArtistCredit {
            parts: vec![
                CreditPart {
                    artist: ArtistId("wes-montgomery".to_string()),
                    name: "Wes Montgomery".to_string(),
                    joinphrase: " & ".to_string(),
                },
                CreditPart {
                    artist: ArtistId("wynton-kelly-trio".to_string()),
                    name: "Wynton Kelly Trio".to_string(),
                    joinphrase: String::new(),
                },
            ],
        };
        assert_eq!(credit.display(), "Wes Montgomery & Wynton Kelly Trio");
        assert!(!credit.is_various_artists());

        let various = ArtistCredit {
            parts: vec![CreditPart {
                artist: ArtistId(VARIOUS_ARTISTS_ID.to_string()),
                name: "Various Artists".to_string(),
                joinphrase: String::new(),
            }],
        };
        assert!(various.is_various_artists());
    }

    #[test]
    fn describes_release_groups() {
        let cream = ReleaseGroup {
            id: ReleaseGroupId("976d6745-0091-39f1-ae60-c142986485a5".to_string()),
            title: "The Cream of Clapton".to_string(),
            primary: Some(PrimaryType::Album),
            secondary: vec![SecondaryType::Compilation],
            first_release: PartialDate::parse("1995-03-13"),
            credit: ArtistCredit::default(),
        };
        assert_eq!(
            cream.describe(),
            "The Cream of Clapton (Album + Compilation, 1995)"
        );

        let untyped = ReleaseGroup {
            primary: None,
            secondary: vec![],
            first_release: None,
            ..cream
        };
        assert_eq!(untyped.describe(), "The Cream of Clapton");
    }

    #[test]
    fn lucene_quotes_and_escapes_phrases() {
        let query = RecordingQuery {
            title: "Wild Horses".to_string(),
            artist: "The Rolling Stones".to_string(),
            release: Some("Sticky Fingers".to_string()),
        };
        assert_eq!(
            query.lucene(),
            r#"recording:"Wild Horses" AND artist:"The Rolling Stones" AND release:"Sticky Fingers""#
        );

        let quoted = RecordingQuery {
            title: r#"12" Mix \ Dub"#.to_string(),
            artist: "Bobby \"Blue\" Bland".to_string(),
            release: None,
        };
        assert_eq!(
            quoted.lucene(),
            r#"recording:"12\" Mix \\ Dub" AND artist:"Bobby \"Blue\" Bland""#
        );
    }

    #[test]
    fn user_agent_requires_contact() {
        assert_eq!(
            UserAgent::new("scrobble-scrubber/0.2", "ivanmalison@gmail.com")
                .unwrap()
                .as_str(),
            "scrobble-scrubber/0.2 ( ivanmalison@gmail.com )"
        );
        assert!(UserAgent::new("scrobble-scrubber/0.2", " ").is_err());
    }
}
