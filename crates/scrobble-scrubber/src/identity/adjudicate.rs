//! LLM disambiguation. The model chooses an option id; it never authors strings. Every
//! call is audited in the resolution log and keyed by the evidence hash.

use super::llm::{ChoiceRequest, LlmBackend, RawChoice};
use super::outcome::{Outcome, Proof};
use crate::subject::ScrobbleKey;
use sha2::{Digest, Sha256};
use std::fmt::Write;

/// Bump when the prompt or option rendering changes; recorded with every verdict.
pub(crate) const PROMPT_VERSION: u32 = 1;

pub(crate) const MAX_REASON_CHARS: usize = 300;
pub(crate) const MAX_RAW_BYTES: usize = 4096;

pub(crate) const SYSTEM_PROMPT: &str = "\
You help correct a music listening history (last.fm scrobbles) against MusicBrainz.

MusicBrainz terms:
- Recording: one specific captured performance or mix. A studio take, a live take, and a remix of \
the same song are different recordings. MusicBrainz sometimes holds unmerged duplicate entries for \
one recording.
- ISRC: a code assigned to a recording's audio. Two recording entries that share an ISRC are \
normally the same audio.
- Work: the composition. Every recording of a song (studio, live, cover) links to the same work; \
the link carries attributes such as \"live\" or \"cover\".
- Release group: one album, single, or EP across all its editions, with a type such as \
\"Album\", \"Album + Compilation\", \"Album + Live\", \"Album + Remix\", or \"Album + Soundtrack\".

The user message describes one scrobble as the user's player reported it (artist, title, album), \
the same title under the user's other albums, and lettered options. Each option is a candidate \
identity for the scrobble, with its evidence: the kind of proof linking the scrobble to it, the \
title and album the scrobble would be edited to, the release group the scrobbled album matched, \
the canonical release group, and the recordings with their lengths, ISRCs, and work attributes. \
Earlier human decisions on this artist, when present, show what the user accepted or rejected.

Choose:
- an option letter when the evidence shows the scrobble is that recording and its target is right;
- \"keep\" when the scrobble is already right as it is. Prefer \"keep\" when the scrobbled album is \
itself a live, remix, or soundtrack release of a different performance than the option's;
- \"unsure\" when the evidence does not settle it. Answer \"unsure\" rather than guess: a person \
reviews unsure cases.

Reply with JSON only: {\"choice\": \"<option letter, keep, or unsure>\", \"confidence\": \
\"high\" | \"medium\" | \"low\", \"reason\": \"<one or two sentences>\"}";

/// 'A'..='F'.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OptionId(pub char);

/// Everything an adjudicator (LLM or the human walk) sees.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Case {
    pub key: ScrobbleKey,
    /// Live plays of this key when the case was built.
    pub plays: u32,
    /// The same artist + title under other albums in the library.
    pub siblings: Vec<ScrobbleKey>,
    /// 2..=6 outcome-distinct options, best deterministic proof first.
    pub options: Vec<CaseOption>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CaseOption {
    pub id: OptionId,
    pub outcome: Outcome,
    pub evidence: OptionEvidence,
}

/// Facts rendered into the prompt and `identity show`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OptionEvidence {
    /// (mbid, title, disambiguation, length ms)
    pub recordings: Vec<(String, String, String, Option<u32>)>,
    pub isrcs: Vec<String>,
    /// Work relationship attributes: "live", "cover", ...
    pub work_attributes: Vec<String>,
    /// "The Cream of Clapton (Album + Compilation, 1995)"
    pub scrobbled_group: Option<String>,
    /// "Slowhand (Album, 1977)"
    pub canonical_group: Option<String>,
}

impl Case {
    /// Assigns option ids in order. Panics on fewer than 2 or more than 6 options (the
    /// resolver guarantees the bound).
    pub(crate) fn new(
        key: ScrobbleKey,
        plays: u32,
        siblings: Vec<ScrobbleKey>,
        options: Vec<(Outcome, OptionEvidence)>,
    ) -> Self {
        assert!(
            (2..=6).contains(&options.len()),
            "a case needs 2..=6 options, got {}",
            options.len()
        );
        let options = ('A'..)
            .zip(options)
            .map(|(id, (outcome, evidence))| CaseOption {
                id: OptionId(id),
                outcome,
                evidence,
            })
            .collect();
        Self {
            key,
            plays,
            siblings,
            options,
        }
    }
}

/// sha256 over (PROMPT_VERSION, the case minus `plays`). Same evidence, same hash, no
/// second call.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CaseHash(pub String);

pub(crate) fn case_hash(case: &Case) -> CaseHash {
    let evidence = Case {
        plays: 0,
        ..case.clone()
    };
    let json = serde_json::to_vec(&evidence).expect("a case serializes");
    let digest = Sha256::new()
        .chain_update(PROMPT_VERSION.to_le_bytes())
        .chain_update(json)
        .finalize();
    CaseHash(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Option(OptionId),
    /// The scrobble is right as it is: settle with no edit.
    Keep,
    Unsure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Verdict {
    pub choice: Choice,
    pub confidence: Confidence,
    pub reason: String,
    /// The model answered outside the allowed set; coerced to `Unsure`.
    pub invalid: bool,
}

/// A human decision on an earlier identity proposal for the same artist, for few-shot
/// context. Derived from the queue by the lane.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Precedent {
    pub key: ScrobbleKey,
    pub proposed_album: Option<String>,
    pub proposed_title: Option<String>,
    pub accepted: bool,
}

pub(crate) struct Adjudication {
    pub case_hash: CaseHash,
    pub verdict: Verdict,
    pub by: String,
    /// Raw model text, truncated to 4 KiB.
    pub raw: String,
}

pub(crate) struct Adjudicator {
    backend: Box<dyn LlmBackend>,
    daily_budget: u32,
}

impl Adjudicator {
    pub fn new(backend: Box<dyn LlmBackend>, daily_budget: u32) -> Self {
        Self {
            backend,
            daily_budget,
        }
    }

    pub fn daily_budget(&self) -> u32 {
        self.daily_budget
    }

    pub fn backend(&self) -> &dyn LlmBackend {
        self.backend.as_ref()
    }

    pub async fn adjudicate(
        &self,
        case: &Case,
        precedents: &[Precedent],
    ) -> Result<Adjudication, AdjudicateError> {
        let allowed = case
            .options
            .iter()
            .map(|option| option.id.0.to_string())
            .chain(["keep".to_string(), "unsure".to_string()])
            .collect();
        let raw = self
            .backend
            .choose(ChoiceRequest {
                system: SYSTEM_PROMPT.to_string(),
                user: render_case(case, precedents),
                allowed,
            })
            .await?;
        Ok(Adjudication {
            case_hash: case_hash(case),
            verdict: validate(&raw, case),
            by: self.backend.label(),
            raw: truncate_bytes(&raw.text, MAX_RAW_BYTES).to_string(),
        })
    }
}

pub(crate) fn render_case(case: &Case, precedents: &[Precedent]) -> String {
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
            let album = quoted_or_none(sibling.album.as_deref());
            let _ = writeln!(out, "  \"{}\" on {album}", sibling.track);
        }
    }

    for option in &case.options {
        render_option(&mut out, option);
    }

    if !precedents.is_empty() {
        let _ = writeln!(out, "\nEarlier human decisions on this artist:");
        for precedent in precedents {
            let album = quoted_or_none(precedent.key.album.as_deref());
            let mut proposed = Vec::new();
            if let Some(title) = &precedent.proposed_title {
                proposed.push(format!("title \"{title}\""));
            }
            if let Some(album) = &precedent.proposed_album {
                proposed.push(format!("album \"{album}\""));
            }
            let proposed = proposed.join(", ");
            let decision = if precedent.accepted {
                "accepted"
            } else {
                "rejected"
            };
            let _ = writeln!(
                out,
                "  \"{}\" on {album}, proposed {proposed}: {decision}",
                precedent.key.track
            );
        }
    }

    let letters: Vec<String> = case
        .options
        .iter()
        .map(|option| option.id.0.to_string())
        .collect();
    let _ = write!(
        out,
        "\nAnswer with one of: {}, keep, unsure.",
        letters.join(", ")
    );
    out
}

fn render_option(out: &mut String, option: &CaseOption) {
    let outcome = &option.outcome;
    let evidence = &option.evidence;
    let _ = writeln!(out, "\nOption {}", option.id.0);
    let album = match &outcome.canonical {
        Some(canonical) => format!("album \"{}\"", canonical.group.title),
        None => "album unchanged".to_string(),
    };
    let _ = writeln!(out, "  target: title \"{}\", {album}", outcome.title);
    let _ = writeln!(out, "  proof: {}", describe_proof(&outcome.proof));
    if let Some(group) = &evidence.scrobbled_group {
        let _ = writeln!(out, "  scrobbled release group: {group}");
    }
    if let Some(group) = &evidence.canonical_group {
        let _ = writeln!(out, "  canonical release group: {group}");
    }
    let _ = writeln!(out, "  recordings:");
    for (mbid, title, disambiguation, length_ms) in &evidence.recordings {
        let _ = write!(out, "    {mbid} \"{title}\"");
        if !disambiguation.is_empty() {
            let _ = write!(out, " ({disambiguation})");
        }
        let _ = writeln!(out, " {}", describe_length(*length_ms));
    }
    let _ = writeln!(out, "  ISRCs: {}", list_or_none(&evidence.isrcs));
    let _ = writeln!(
        out,
        "  work attributes: {}",
        list_or_none(&evidence.work_attributes)
    );
}

fn describe_proof(proof: &Proof) -> String {
    match proof {
        Proof::SameRecording => "same recording".to_string(),
        Proof::SharedIsrc { isrc, len_delta_ms } => format!(
            "different recording entries sharing ISRC {isrc}, lengths {} s apart",
            tenths_of_seconds(*len_delta_ms)
        ),
        Proof::SameWork => "same work, different recording, no shared ISRC".to_string(),
        Proof::TitleOnly => "title and artist match only, no album to anchor on".to_string(),
        Proof::Researched { .. } => "found by LLM research, verified on MusicBrainz".to_string(),
    }
}

fn describe_length(length_ms: Option<u32>) -> String {
    match length_ms {
        Some(ms) => {
            let seconds = ms / 1000;
            format!("{}:{:02}", seconds / 60, seconds % 60)
        }
        None => "length unknown".to_string(),
    }
}

fn tenths_of_seconds(ms: u32) -> String {
    format!("{}.{}", ms / 1000, ms % 1000 / 100)
}

fn quoted_or_none(value: Option<&str>) -> String {
    value.map_or_else(|| "no album".to_string(), |value| format!("\"{value}\""))
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

/// Maps raw model output onto the allowed set; anything else is `Unsure` + `invalid`.
pub(crate) fn validate(raw: &RawChoice, case: &Case) -> Verdict {
    let answer = raw.choice.trim();
    let choice = if answer.eq_ignore_ascii_case("keep") {
        Some(Choice::Keep)
    } else if answer.eq_ignore_ascii_case("unsure") {
        Some(Choice::Unsure)
    } else {
        case.options
            .iter()
            .find(|option| answer.eq_ignore_ascii_case(&option.id.0.to_string()))
            .map(|option| Choice::Option(option.id))
    };
    Verdict {
        invalid: choice.is_none(),
        choice: choice.unwrap_or(Choice::Unsure),
        confidence: parse_confidence(&raw.confidence),
        reason: raw.reason.trim().chars().take(MAX_REASON_CHARS).collect(),
    }
}

pub(crate) fn parse_confidence(raw: &str) -> Confidence {
    match raw.trim().to_ascii_lowercase().as_str() {
        "high" => Confidence::High,
        "medium" => Confidence::Medium,
        _ => Confidence::Low,
    }
}

pub(crate) fn truncate_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[derive(Debug, thiserror::Error)]
pub enum AdjudicateError {
    #[error("llm transport: {0}")]
    Transport(String),
    #[error("llm rate limited")]
    RateLimited,
    #[error("{0} cannot do web research")]
    Unsupported(String),
}

#[cfg(test)]
#[path = "adjudicate_tests.rs"]
mod tests;
