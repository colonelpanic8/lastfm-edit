//! The last resort for keys the resolver can't settle: an LLM with web search names a
//! MusicBrainz recording (and release group). It answers with identifiers only;
//! [`Resolver::verify_research`](super::resolve::Resolver::verify_research) checks them
//! against MusicBrainz and builds the outcome from MusicBrainz data.

use super::adjudicate::{
    parse_confidence, truncate_bytes, AdjudicateError, CaseOption, Confidence, MAX_RAW_BYTES,
};
use super::llm::{LlmBackend, ResearchRequest};
use super::mb::{RecordingId, ReleaseGroupId};
use super::outcome::Outcome;
use crate::subject::ScrobbleKey;
use serde_json::json;
use std::fmt::Write;

const MAX_REASON_CHARS: usize = 600;

pub(crate) const SYSTEM_PROMPT: &str = "\
You help correct a music listening history (last.fm scrobbles) against MusicBrainz. A \
deterministic resolver could not settle the scrobble in the user message; the message says what \
it tried.

Find:
- recording_mbid: the MusicBrainz recording this scrobble most likely is. A studio take, a live \
take, and a remix of the same song are different recordings; prefer the one on the release the \
scrobbled album names.
- release_group_mbid: the MusicBrainz release group whose title matches the scrobbled album \
string and which contains that recording. Leave it empty when there is no album or none matches.

How:
- Search the web, and fetch musicbrainz.org pages or the MusicBrainz API, for example \
https://musicbrainz.org/ws/2/recording?query=recording:%22Title%22%20AND%20artist:%22Artist%22&fmt=json \
or https://musicbrainz.org/ws/2/recording/<mbid>?inc=artist-credits+releases+release-groups&fmt=json. \
Be polite to MusicBrainz: a few fetches, not dozens.
- last.fm artist names often differ from MusicBrainz credits (\"Bill Evans\" for \"Bill Evans \
Trio\", a dropped \"The\", a featured artist moved into the title), and titles and albums may carry \
remaster or edition noise.
- Give only MBIDs you actually saw on musicbrainz.org. Never construct or guess one.
- If you can't find it, answer with empty strings and low confidence instead of guessing. \
MusicBrainz checks your answer and a person reviews it.

Reply with JSON only: {\"recording_mbid\": \"<uuid or empty>\", \"release_group_mbid\": \
\"<uuid or empty>\", \"confidence\": \"high\" | \"medium\" | \"low\", \"reason\": \"<one to three \
sentences>\"}";

pub(crate) fn answer_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["recording_mbid", "release_group_mbid", "confidence", "reason"],
        "properties": {
            "recording_mbid": {"type": "string"},
            "release_group_mbid": {"type": "string"},
            "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
            "reason": {"type": "string"},
        },
    })
}

/// What the resolver concluded before research, shown to the model.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Prior {
    /// No search hit passed the title, artist and version filters.
    NotFound { queries: Vec<String> },
    /// Several candidates, and the choice LLM was unsure among them.
    Unsure {
        options: Vec<CaseOption>,
        reason: String,
    },
    /// A title + artist match whose releases include nothing named like the album.
    Unanchored { outcome: Box<Outcome> },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResearchCase {
    pub key: ScrobbleKey,
    pub plays: u32,
    /// The same artist + title under other albums in the library.
    pub siblings: Vec<ScrobbleKey>,
    pub prior: Prior,
}

/// A validated answer: ids are UUIDs or absent; not yet checked against MusicBrainz.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Research {
    pub recording: Option<RecordingId>,
    pub release_group: Option<ReleaseGroupId>,
    pub confidence: Confidence,
    pub reason: String,
    pub by: String,
    /// Raw model output, truncated to 4 KiB.
    pub raw: String,
}

pub(crate) struct Researcher {
    backend: Box<dyn LlmBackend>,
    daily_budget: u32,
}

impl Researcher {
    pub fn new(backend: Box<dyn LlmBackend>, daily_budget: u32) -> Self {
        Self {
            backend,
            daily_budget,
        }
    }

    pub fn daily_budget(&self) -> u32 {
        self.daily_budget
    }

    pub async fn research(&self, case: &ResearchCase) -> Result<Research, AdjudicateError> {
        let raw = self
            .backend
            .research(ResearchRequest {
                system: SYSTEM_PROMPT.to_string(),
                user: render_case(case),
                schema: answer_schema(),
            })
            .await?;
        Ok(Research {
            recording: raw.recording,
            release_group: raw.release_group,
            confidence: parse_confidence(&raw.confidence),
            reason: raw.reason.trim().chars().take(MAX_REASON_CHARS).collect(),
            by: self.backend.label(),
            raw: truncate_bytes(&raw.text, MAX_RAW_BYTES).to_string(),
        })
    }
}

/// Deterministic prompt text for a case.
pub(crate) fn render_case(case: &ResearchCase) -> String {
    let mut out = String::new();
    let key = &case.key;
    let _ = writeln!(out, "Scrobble");
    let _ = writeln!(out, "  artist: \"{}\"", key.artist);
    let _ = writeln!(out, "  title: \"{}\"", key.track);
    let _ = writeln!(out, "  album: {}", quoted_or_none(key.album.as_deref()));
    let _ = writeln!(out, "  plays: {}", case.plays);
    if case.siblings.is_empty() {
        let _ = writeln!(out, "Same title under the user's other albums: none");
    } else {
        let _ = writeln!(out, "Same title under the user's other albums:");
        for sibling in &case.siblings {
            let _ = writeln!(out, "  {}", quoted_or_none(sibling.album.as_deref()));
        }
    }

    let _ = writeln!(out, "\nWhat the resolver tried:");
    match &case.prior {
        Prior::NotFound { queries } => {
            let _ = writeln!(
                out,
                "  These MusicBrainz recording searches found no recording whose title, artist \
                 credit and version all match:"
            );
            for query in queries {
                let _ = writeln!(out, "    {query}");
            }
        }
        Prior::Unsure { options, reason } => {
            let _ = writeln!(
                out,
                "  It found several candidates and another model could not choose: \"{reason}\""
            );
            for option in options {
                let evidence = &option.evidence;
                let _ = writeln!(out, "  Option {}:", option.id.0);
                for (mbid, title, disambiguation, _) in &evidence.recordings {
                    let note = if disambiguation.is_empty() {
                        String::new()
                    } else {
                        format!(" ({disambiguation})")
                    };
                    let _ = writeln!(out, "    recording {mbid} \"{title}\"{note}");
                }
                if let Some(group) = &evidence.scrobbled_group {
                    let _ = writeln!(out, "    on {group}");
                }
                if let Some(group) = &evidence.canonical_group {
                    let _ = writeln!(out, "    canonical album {group}");
                }
            }
        }
        Prior::Unanchored { outcome } => {
            let recording = outcome.master.first().map_or("", |id| id.0.as_str());
            let _ = match &outcome.scrobbled_group {
                None => writeln!(
                    out,
                    "  It matched recording {recording} \"{}\" on title and artist alone: none \
                     of that recording's releases is named like the scrobbled album.",
                    outcome.title
                ),
                Some(group) => writeln!(
                    out,
                    "  It matched recording {recording} \"{}\" on {}, but MusicBrainz found \
                     more recordings than one search returns, so others went unchecked.",
                    outcome.title,
                    group.describe()
                ),
            };
        }
    }
    let _ = write!(
        out,
        "\nAnswer with the MusicBrainz recording and release group ids, or empty strings."
    );
    out
}

fn quoted_or_none(value: Option<&str>) -> String {
    value.map_or_else(|| "no album".to_string(), |value| format!("\"{value}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::llm::scripted::ScriptedBackend;
    use crate::identity::llm::RawResearch;
    use std::sync::Arc;

    fn evans() -> ResearchCase {
        ResearchCase {
            key: ScrobbleKey {
                artist: "Bill Evans".into(),
                track: "Waltz for Debby".into(),
                album: Some("Waltz For Debby".into()),
            },
            plays: 14,
            siblings: vec![ScrobbleKey {
                artist: "Bill Evans".into(),
                track: "Waltz for Debby".into(),
                album: Some("Portrait in Jazz".into()),
            }],
            prior: Prior::NotFound {
                queries: vec![
                    r#"recording:"Waltz for Debby" AND artist:"Bill Evans" AND release:"Waltz For Debby""#
                        .into(),
                    r#"recording:"Waltz for Debby" AND artist:"Bill Evans""#.into(),
                ],
            },
        }
    }

    #[test]
    fn renders_a_not_found_case() {
        let expected = r#"Scrobble
  artist: "Bill Evans"
  title: "Waltz for Debby"
  album: "Waltz For Debby"
  plays: 14
Same title under the user's other albums:
  "Portrait in Jazz"

What the resolver tried:
  These MusicBrainz recording searches found no recording whose title, artist credit and version all match:
    recording:"Waltz for Debby" AND artist:"Bill Evans" AND release:"Waltz For Debby"
    recording:"Waltz for Debby" AND artist:"Bill Evans"

Answer with the MusicBrainz recording and release group ids, or empty strings."#;
        assert_eq!(render_case(&evans()), expected);
    }

    #[tokio::test]
    async fn research_sends_the_case_and_caps_what_it_keeps() {
        let backend = Arc::new(ScriptedBackend::default());
        backend.research_answers.lock().unwrap().push(RawResearch {
            recording: Some(RecordingId("d31d4528-14cf-48d9-960d-6a07eae19760".into())),
            release_group: None,
            confidence: "Medium".into(),
            reason: format!("  {}  ", "é".repeat(700)),
            text: "ü".repeat(2100),
        });
        let researcher = Researcher::new(Box::new(backend.clone()), 20);

        let research = researcher.research(&evans()).await.unwrap();

        assert_eq!(
            research,
            Research {
                recording: Some(RecordingId("d31d4528-14cf-48d9-960d-6a07eae19760".into())),
                release_group: None,
                confidence: Confidence::Medium,
                reason: "é".repeat(600),
                by: "scripted".into(),
                raw: "ü".repeat(2048),
            }
        );
        assert_eq!(
            backend.research_requests.lock().unwrap().as_slice(),
            [ResearchRequest {
                system: SYSTEM_PROMPT.into(),
                user: render_case(&evans()),
                schema: answer_schema(),
            }]
        );
    }
}
