//! Deterministic evidence gathering: scrobble key -> [`Resolution`]. No LLM here.

use super::adjudicate::OptionEvidence;
use super::canonical::{choose_canonical, Canonical, Rung};
use super::mb::{
    ArtistCredit, Isrc, MbError, MbLookup, PrimaryType, RecordingDetail, RecordingHit, RecordingId,
    RecordingQuery, ReleaseGroup, ReleaseGroupId, ReleaseRef, SecondaryType,
};
use super::outcome::{Outcome, Proof};
use super::title::{
    edition_names_single_or_ep, fold, parse_title, strip_edition, IdentityMarker, ParsedTitle,
};
use crate::subject::ScrobbleKey;

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Resolution {
    Resolved(Outcome),
    /// 2..=6 outcome-distinct options, best proof first.
    Ambiguous(Vec<(Outcome, OptionEvidence)>),
    NotFound {
        queries: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct ResolveLimits {
    pub min_score: u8,
    /// Recordings per master (ISRC cluster).
    pub max_master_size: usize,
    /// Releases browsed per recording.
    pub max_releases: usize,
    /// Recordings browsed per work when looking for a studio sibling.
    pub max_work_recordings: usize,
    pub isrc_len_tolerance_ms: u32,
}

impl Default for ResolveLimits {
    fn default() -> Self {
        Self {
            min_score: 80,
            max_master_size: 8,
            max_releases: 300,
            max_work_recordings: 50,
            isrc_len_tolerance_ms: 5_000,
        }
    }
}

const MAX_OPTIONS: usize = 6;

/// Work relationships whose recordings are not interchangeable with a studio take.
const NON_STUDIO_WORK_ATTRIBUTES: [&str; 3] = ["live", "medley", "partial"];

pub(crate) struct Resolver<'a> {
    pub mb: &'a dyn MbLookup,
    pub limits: ResolveLimits,
}

/// One possible conclusion for the key, with the facts an adjudicator would see.
struct Candidate {
    outcome: Outcome,
    evidence: OptionEvidence,
    /// A browse or ISRC cluster hit its cap, so a better canonical may be missing.
    capped: bool,
}

struct Search {
    /// Plausible hits of the first query that had any, with their scrobbled groups.
    hits: Vec<(RecordingHit, Option<ReleaseGroup>)>,
    queries: Vec<String>,
    /// A query so far matched more recordings than MB returned.
    truncated: bool,
}

/// A recording proven to be the same master as the matched one.
struct Member {
    detail: RecordingDetail,
    /// The ISRC linking it to the matched recording; `None` for the matched recording.
    isrc: Option<Isrc>,
    releases: Vec<ReleaseRef>,
}

impl Resolver<'_> {
    /// Outcomes are grouped by the edit they imply: one group resolves, several are
    /// ambiguous, none is not-found. A truncated browse or oversized master also offers
    /// staying put instead of guessing a move.
    ///
    /// A search MB matched more recordings for than it returned can't rule out a better
    /// match, so its outcomes are demoted to [`Proof::TitleOnly`]: never auto-applied.
    pub async fn resolve(&self, key: &ScrobbleKey) -> Result<Resolution, MbError> {
        let parsed = parse_title(&key.track);
        let search = self.search(key, &parsed).await?;
        let mut candidates = Vec::new();
        for (hit, scrobbled) in search.hits {
            candidates.extend(self.candidates(hit, scrobbled).await?);
        }
        if search.truncated {
            for candidate in &mut candidates {
                candidate.outcome.proof = Proof::TitleOnly;
            }
        }
        Ok(conclude(candidates, search.queries))
    }

    async fn search(&self, key: &ScrobbleKey, parsed: &ParsedTitle) -> Result<Search, MbError> {
        let mut search = Search {
            hits: Vec::new(),
            queries: Vec::new(),
            truncated: false,
        };
        for query in ladder(key, parsed) {
            search.queries.push(query.lucene());
            let (hits, truncated) = self.mb.search_recordings(&query).await?;
            search.truncated |= truncated.0;
            search.hits = self.plausible(hits, key, parsed);
            if !search.hits.is_empty() {
                break;
            }
        }
        Ok(search)
    }

    /// Checks an LLM research answer against MusicBrainz and builds the outcome the
    /// resolver would have built for that recording, with a [`Proof::Researched`] proof.
    /// `None` unless the recording exists and plausibly is the scrobble: fold-equal base
    /// title, an artist credit that leniently matches, consistent version markers.
    /// `release_group` only chooses among the recording's official release groups named
    /// like the scrobbled album; one it isn't on, or named otherwise, is ignored.
    pub async fn verify_research(
        &self,
        key: &ScrobbleKey,
        recording: &RecordingId,
        release_group: Option<&ReleaseGroupId>,
        by: &str,
    ) -> Result<Option<Outcome>, MbError> {
        let Some(detail) = found(self.mb.recording(recording).await)? else {
            return Ok(None);
        };
        let parsed = parse_title(&key.track);
        if !title_matches(&parsed, &detail.title) || !artist_plausible(&key.artist, &detail.credit)
        {
            return Ok(None);
        }
        let (releases, _) = self
            .mb
            .recording_releases(&detail.id, self.limits.max_releases)
            .await?;
        let scrobbled = key.album.as_deref().and_then(|album| {
            let named: Vec<ReleaseRef> = releases
                .iter()
                .filter(|r| Some(&r.group.id) == release_group)
                .cloned()
                .collect();
            scrobbled_group(&named, album).or_else(|| scrobbled_group(&releases, album))
        });
        let hit = RecordingHit {
            id: detail.id,
            score: 100,
            title: detail.title,
            disambiguation: detail.disambiguation,
            length_ms: detail.length_ms,
            credit: detail.credit,
            snippet_releases: releases,
        };
        if !markers_consistent(&parsed, &hit, scrobbled.as_ref()) {
            return Ok(None);
        }
        let Some(candidate) = self.candidates(hit, scrobbled).await?.into_iter().next() else {
            return Ok(None);
        };
        let mut outcome = candidate.outcome;
        if candidate.capped {
            outcome.canonical = None;
        }
        outcome.proof = Proof::Researched { by: by.to_string() };
        Ok(Some(outcome))
    }

    /// Hits that could be this scrobble, each with the release group its album matched.
    /// When any hit matched the album, hits that didn't are dropped: the album string is
    /// the strongest identity evidence a scrobble carries.
    fn plausible(
        &self,
        hits: Vec<RecordingHit>,
        key: &ScrobbleKey,
        parsed: &ParsedTitle,
    ) -> Vec<(RecordingHit, Option<ReleaseGroup>)> {
        let mut kept: Vec<_> = hits
            .into_iter()
            .filter(|hit| hit.score >= self.limits.min_score)
            .filter(|hit| title_matches(parsed, &hit.title))
            .filter(|hit| artist_matches(&key.artist, &hit.credit))
            .map(|hit| {
                let scrobbled = key
                    .album
                    .as_deref()
                    .and_then(|album| scrobbled_group(&hit.snippet_releases, album));
                (hit, scrobbled)
            })
            .filter(|(hit, scrobbled)| markers_consistent(parsed, hit, scrobbled.as_ref()))
            .collect();
        if kept.iter().any(|(_, scrobbled)| scrobbled.is_some()) {
            kept.retain(|(_, scrobbled)| scrobbled.is_some());
        }
        kept
    }

    async fn candidates(
        &self,
        hit: RecordingHit,
        scrobbled: Option<ReleaseGroup>,
    ) -> Result<Vec<Candidate>, MbError> {
        match scrobbled {
            None => Ok(vec![title_only(hit)]),
            Some(group) if is_plain_album(&group) => Ok(vec![plain_album(hit, group)]),
            Some(group) => self.expand(hit, group).await,
        }
    }

    /// Builds the master, browses its releases, and chooses where the recording belongs.
    async fn expand(
        &self,
        hit: RecordingHit,
        scrobbled: ReleaseGroup,
    ) -> Result<Vec<Candidate>, MbError> {
        let Some(matched) = found(self.mb.recording(&hit.id).await)? else {
            return Ok(Vec::new());
        };
        let (mut master, mut capped) = self.master(matched).await?;
        for member in &mut master {
            let (releases, truncated) = self
                .mb
                .recording_releases(&member.detail.id, self.limits.max_releases)
                .await?;
            capped |= truncated.0;
            member.releases = releases;
        }
        let matched = &master[0].detail;
        let all_releases: Vec<ReleaseRef> = master
            .iter()
            .flat_map(|member| member.releases.iter().cloned())
            .collect();
        let canonical = choose_canonical(&all_releases, &matched.credit, &matched.title);

        let stays = canonical
            .as_ref()
            .is_none_or(|canonical| canonical.group.id == scrobbled.id);
        if stays && scrobbled.secondary.contains(&SecondaryType::Compilation) {
            if let Some(candidate) = self.same_work(&master, &scrobbled).await? {
                return Ok(vec![candidate]);
            }
        }

        let proof = match &canonical {
            Some(canonical) => master_proof(&master, &canonical.group.id),
            None => Proof::SameRecording,
        };
        let outcome = Outcome {
            master: master.iter().map(|m| m.detail.id.clone()).collect(),
            title: matched.title.clone(),
            scrobbled_group: Some(scrobbled),
            canonical,
            proof,
        };
        Ok(vec![Candidate {
            evidence: evidence(master.iter().map(|m| &m.detail), matched),
            outcome,
            capped,
        }])
    }

    /// The matched recording plus its same-ISRC siblings (see [`is_isrc_sibling`]).
    async fn master(&self, matched: RecordingDetail) -> Result<(Vec<Member>, bool), MbError> {
        let isrcs = matched.isrcs.clone();
        let mut master = vec![Member {
            detail: matched,
            isrc: None,
            releases: Vec::new(),
        }];
        let mut capped = false;
        for isrc in isrcs {
            let Some(ids) = found(self.mb.isrc_members(&isrc).await)? else {
                continue;
            };
            for id in ids {
                if master.iter().any(|m| m.detail.id == id) {
                    continue;
                }
                if master.len() >= self.limits.max_master_size {
                    capped = true;
                    break;
                }
                let Some(sibling) = found(self.mb.recording(&id).await)? else {
                    continue;
                };
                if is_isrc_sibling(&master[0].detail, &sibling, self.limits)
                    && !master.iter().any(|m| m.detail.id == sibling.id)
                {
                    master.push(Member {
                        detail: sibling,
                        isrc: Some(isrc.clone()),
                        releases: Vec::new(),
                    });
                }
            }
        }
        Ok((master, capped))
    }

    /// A compilation-only recording with no ISRC link to a studio master: look for the
    /// same artist's plain recording of the same work and where that one belongs.
    async fn same_work(
        &self,
        master: &[Member],
        scrobbled: &ReleaseGroup,
    ) -> Result<Option<Candidate>, MbError> {
        let matched = &master[0].detail;
        let works = matched.works.iter().filter(|link| {
            !link
                .attributes
                .iter()
                .any(|a| NON_STUDIO_WORK_ATTRIBUTES.contains(&a.as_str()))
        });
        for link in works {
            let (ids, _) = self
                .mb
                .work_recordings(&link.work, self.limits.max_work_recordings)
                .await?;
            for id in ids {
                if master.iter().any(|m| m.detail.id == id) {
                    continue;
                }
                let Some(sibling) = found(self.mb.recording(&id).await)? else {
                    continue;
                };
                if !is_plain_studio_take(matched, &sibling) {
                    continue;
                }
                let (releases, _) = self
                    .mb
                    .recording_releases(&sibling.id, self.limits.max_releases)
                    .await?;
                let Some(canonical) = choose_canonical(&releases, &sibling.credit, &sibling.title)
                else {
                    continue;
                };
                if canonical.group.id == scrobbled.id {
                    continue;
                }
                return Ok(Some(Candidate {
                    evidence: evidence([matched, &sibling], matched),
                    outcome: Outcome {
                        master: vec![matched.id.clone(), sibling.id.clone()],
                        title: matched.title.clone(),
                        scrobbled_group: Some(scrobbled.clone()),
                        canonical: Some(canonical),
                        proof: Proof::SameWork,
                    },
                    capped: false,
                }));
            }
        }
        Ok(None)
    }
}

/// `NotFound` from a lookup is an absent fact, not a failure.
fn found<T>(result: Result<T, MbError>) -> Result<Option<T>, MbError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(MbError::NotFound) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Searches to try in order: the parsed base title with the edition-stripped album, then
/// without the album, then the raw title.
fn ladder(key: &ScrobbleKey, parsed: &ParsedTitle) -> Vec<RecordingQuery> {
    let query = |title: &str, release: Option<String>| RecordingQuery {
        title: title.to_string(),
        artist: key.artist.clone(),
        release,
    };
    let mut ladder = Vec::new();
    if let Some(album) = &key.album {
        ladder.push(query(&parsed.base, Some(strip_edition(album))));
    }
    ladder.push(query(&parsed.base, None));
    ladder.push(query(&parsed.raw, None));
    ladder.dedup();
    ladder
}

/// The Lucene queries [`Resolver::resolve`] tries for `key`, in order.
pub(crate) fn queries(key: &ScrobbleKey) -> Vec<String> {
    ladder(key, &parse_title(&key.track))
        .iter()
        .map(RecordingQuery::lucene)
        .collect()
}

fn title_matches(parsed: &ParsedTitle, hit_title: &str) -> bool {
    let base = fold(&parsed.base);
    base == fold(hit_title)
        || base == fold(&parse_title(hit_title).base)
        || fold(&parsed.raw) == fold(hit_title)
}

/// Words that make a credited name an ensemble led by the scrobbled artist: "Bill Evans
/// Trio", "Bob Marley & The Wailers" (`fold` spells "&" as "and").
const ENSEMBLE_WORDS: [&str; 8] = [
    "trio",
    "quartet",
    "quintet",
    "sextet",
    "band",
    "orchestra",
    "group",
    "and",
];

/// The scrobbled artist is the whole credit, any credited artist, the primary artist
/// followed by a featured/joined tail ("Artist feat. X", "Artist & Y"), or the leader of
/// the primary ensemble ("Bill Evans" for "Bill Evans Trio").
fn artist_matches(artist: &str, credit: &ArtistCredit) -> bool {
    let artist = fold(artist);
    if artist == fold(&credit.display()) || credit.parts.iter().any(|p| fold(&p.name) == artist) {
        return true;
    }
    let Some(primary) = credit.parts.first() else {
        return false;
    };
    let primary = fold(&primary.name);
    let leads_ensemble = primary
        .strip_prefix(&artist)
        .and_then(|tail| tail.strip_prefix(' '))
        .and_then(|tail| tail.split(' ').next())
        .is_some_and(|word| ENSEMBLE_WORDS.contains(&word));
    leads_ensemble
        || artist.strip_prefix(&primary).is_some_and(|tail| {
            [
                " feat",
                " ft",
                " featuring ",
                " and ",
                " with ",
                ", ",
                " x ",
                " vs",
            ]
            .iter()
            .any(|joiner| tail.starts_with(joiner))
        })
}

/// Research names one recording, so its credit only has to be close: [`artist_matches`],
/// or a credited name that extends the scrobbled artist by whole words, or the reverse.
fn artist_plausible(artist: &str, credit: &ArtistCredit) -> bool {
    let folded = fold(artist);
    artist_matches(artist, credit)
        || credit.parts.iter().any(|part| {
            let name = fold(&part.name);
            name.starts_with(&format!("{folded} ")) || folded.starts_with(&format!("{name} "))
        })
}

/// The release group a scrobbled album string names, preferring an exact title match over
/// an edition-stripped one. A stripped " - Single" or " - EP" only names a group of that
/// type: "Purple Rain - Single" is not the album "Purple Rain".
fn scrobbled_group(releases: &[ReleaseRef], album: &str) -> Option<ReleaseGroup> {
    let exact = fold(album);
    let stripped = fold(&strip_edition(album));
    let single_or_ep = edition_names_single_or_ep(album);
    let titled = |want: &str, typed: &dyn Fn(&ReleaseGroup) -> bool| {
        releases
            .iter()
            .find(|r| (fold(&r.title) == want || fold(&r.group.title) == want) && typed(&r.group))
    };
    titled(&exact, &|_| true)
        .or_else(|| {
            titled(&stripped, &|group| {
                !single_or_ep
                    || matches!(group.primary, Some(PrimaryType::Single | PrimaryType::Ep))
            })
        })
        .map(|r| r.group.clone())
}

/// Identity markers on the scrobble must show up on the hit, and the hit's
/// [`version_markers`] must show up on the scrobble. A scrobbled album that is a Live
/// release group counts as a Live marker; a hit found only on Live release groups counts
/// as live.
fn markers_consistent(
    parsed: &ParsedTitle,
    hit: &RecordingHit,
    scrobbled: Option<&ReleaseGroup>,
) -> bool {
    let scrobbled_live = scrobbled.is_some_and(|g| g.secondary.contains(&SecondaryType::Live));
    let mut scrobble_markers = normalized(&parsed.identity);
    if scrobbled_live {
        scrobble_markers.push(IdentityMarker::Live);
    }

    let mut hit_markers = version_markers(&hit.title, &hit.disambiguation);
    let disambiguation = disambiguation_markers(&hit.disambiguation);
    let snippet_live = !hit.snippet_releases.is_empty()
        && hit
            .snippet_releases
            .iter()
            .all(|r| r.group.secondary.contains(&SecondaryType::Live));
    if snippet_live && scrobbled.is_none() {
        hit_markers.push(IdentityMarker::Live);
    }

    let reflected = |marker: &IdentityMarker| {
        hit_markers.contains(marker)
            || disambiguation.contains(marker)
            || (*marker == IdentityMarker::Live && (snippet_live || scrobbled_live))
    };
    parsed
        .identity
        .iter()
        .all(|m| reflected(&normalized_one(m)))
        && hit_markers.iter().all(|m| scrobble_markers.contains(m))
}

/// The versions a recording's title and disambiguation pin down. Free-form disambiguation
/// notes ("original studio mix", "5.1 mix") don't count; ones naming a cut ("2015 remix")
/// do.
fn version_markers(title: &str, disambiguation: &str) -> Vec<IdentityMarker> {
    let mut markers = normalized(&parse_title(title).identity);
    markers.extend(
        disambiguation_markers(disambiguation)
            .into_iter()
            .filter(|m| !matches!(m, IdentityMarker::OtherVersion(_)) || m.names_cut()),
    );
    markers
}

fn disambiguation_markers(disambiguation: &str) -> Vec<IdentityMarker> {
    if disambiguation.is_empty() {
        return Vec::new();
    }
    normalized(&parse_title(&format!("_ ({disambiguation})")).identity)
}

fn normalized(markers: &[IdentityMarker]) -> Vec<IdentityMarker> {
    markers.iter().map(normalized_one).collect()
}

fn normalized_one(marker: &IdentityMarker) -> IdentityMarker {
    match marker {
        IdentityMarker::OtherVersion(note) => IdentityMarker::OtherVersion(fold(note)),
        other => other.clone(),
    }
}

fn is_plain_album(group: &ReleaseGroup) -> bool {
    group.primary == Some(PrimaryType::Album) && group.secondary.is_empty()
}

fn is_plain_studio_take(matched: &RecordingDetail, sibling: &RecordingDetail) -> bool {
    sibling.credit.artist_ids() == matched.credit.artist_ids()
        && fold(&parse_title(&sibling.title).base) == fold(&parse_title(&matched.title).base)
        && version_markers(&sibling.title, &sibling.disambiguation).is_empty()
}

/// ISRCs get misassigned, so a shared code alone doesn't make one master: the recordings
/// must also be the same song and version by an overlapping set of artists, of near-equal
/// length.
fn is_isrc_sibling(
    matched: &RecordingDetail,
    other: &RecordingDetail,
    limits: ResolveLimits,
) -> bool {
    let artists = matched.credit.artist_ids();
    let versions = version_markers(&matched.title, &matched.disambiguation);
    let other_versions = version_markers(&other.title, &other.disambiguation);
    fold(&parse_title(&other.title).base) == fold(&parse_title(&matched.title).base)
        && versions.iter().all(|m| other_versions.contains(m))
        && other_versions.iter().all(|m| versions.contains(m))
        && other
            .credit
            .artist_ids()
            .iter()
            .any(|id| artists.contains(id))
        && len_delta(matched, other).is_some_and(|delta| delta <= limits.isrc_len_tolerance_ms)
}

fn len_delta(a: &RecordingDetail, b: &RecordingDetail) -> Option<u32> {
    Some(a.length_ms?.abs_diff(b.length_ms?))
}

/// `SameRecording` when the canonical group holds the matched recording itself, else the
/// ISRC link of the member that put it there.
fn master_proof(master: &[Member], canonical: &ReleaseGroupId) -> Proof {
    let on_canonical = |member: &Member| member.releases.iter().any(|r| r.group.id == *canonical);
    if on_canonical(&master[0]) {
        return Proof::SameRecording;
    }
    master
        .iter()
        .skip(1)
        .filter(|member| on_canonical(member))
        .filter_map(|member| {
            let isrc = member.isrc.as_ref()?;
            let delta = len_delta(&master[0].detail, &member.detail)?;
            Some((delta, isrc))
        })
        .min_by_key(|(delta, _)| *delta)
        .map(|(len_delta_ms, isrc)| Proof::SharedIsrc {
            isrc: isrc.0.clone(),
            len_delta_ms,
        })
        .unwrap_or(Proof::SameRecording)
}

fn title_only(hit: RecordingHit) -> Candidate {
    Candidate {
        evidence: hit_evidence(&hit),
        outcome: Outcome {
            master: vec![hit.id],
            title: hit.title,
            scrobbled_group: None,
            canonical: None,
            proof: Proof::TitleOnly,
        },
        capped: false,
    }
}

/// Plain albums never move; the group itself is canonical, which can only fix edition
/// wording in the album string.
fn plain_album(hit: RecordingHit, group: ReleaseGroup) -> Candidate {
    Candidate {
        evidence: hit_evidence(&hit),
        outcome: Outcome {
            master: vec![hit.id],
            title: hit.title,
            scrobbled_group: Some(group.clone()),
            canonical: Some(Canonical {
                group,
                rung: Rung::PlainAlbum,
                single_first: false,
            }),
            proof: Proof::SameRecording,
        },
        capped: false,
    }
}

fn hit_evidence(hit: &RecordingHit) -> OptionEvidence {
    OptionEvidence {
        recordings: vec![(
            hit.id.0.clone(),
            hit.title.clone(),
            hit.disambiguation.clone(),
            hit.length_ms,
        )],
        ..OptionEvidence::default()
    }
}

/// Group descriptions are filled in only when the options are shown to an adjudicator.
fn evidence<'a>(
    recordings: impl IntoIterator<Item = &'a RecordingDetail>,
    matched: &RecordingDetail,
) -> OptionEvidence {
    OptionEvidence {
        recordings: recordings
            .into_iter()
            .map(|r| {
                (
                    r.id.0.clone(),
                    r.title.clone(),
                    r.disambiguation.clone(),
                    r.length_ms,
                )
            })
            .collect(),
        isrcs: matched.isrcs.iter().map(|i| i.0.clone()).collect(),
        work_attributes: matched
            .works
            .iter()
            .flat_map(|link| link.attributes.iter().cloned())
            .collect(),
        scrobbled_group: None,
        canonical_group: None,
    }
}

fn moves(outcome: &Outcome) -> bool {
    match (&outcome.canonical, &outcome.scrobbled_group) {
        (Some(canonical), Some(scrobbled)) => canonical.group.id != scrobbled.id,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn proof_rank(proof: &Proof) -> u8 {
    match proof {
        Proof::SameRecording => 0,
        Proof::SharedIsrc { .. } => 1,
        Proof::SameWork => 2,
        Proof::TitleOnly => 3,
        Proof::Researched { .. } => 4,
    }
}

/// What an outcome would write: candidates agreeing on it are one option.
/// Outcomes whose titles differ only typographically imply the same edit: the user's
/// spelling is kept either way.
fn edit_key(outcome: &Outcome) -> (String, Option<ReleaseGroupId>, Option<ReleaseGroupId>) {
    (
        fold(&outcome.title),
        outcome.canonical.as_ref().map(|c| c.group.id.clone()),
        outcome.scrobbled_group.as_ref().map(|g| g.id.clone()),
    )
}

fn conclude(mut candidates: Vec<Candidate>, queries: Vec<String>) -> Resolution {
    // A capped browse may have missed the true canonical, so staying put is always offered.
    let stays: Vec<Candidate> = candidates
        .iter()
        .filter(|c| c.capped && moves(&c.outcome))
        .map(|c| Candidate {
            outcome: Outcome {
                master: c.outcome.master[..1].to_vec(),
                canonical: None,
                proof: Proof::SameRecording,
                ..c.outcome.clone()
            },
            evidence: c.evidence.clone(),
            capped: false,
        })
        .collect();
    candidates.extend(stays);

    let mut options: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        let key = edit_key(&candidate.outcome);
        match options.iter_mut().find(|o| edit_key(&o.outcome) == key) {
            Some(existing) => {
                if proof_rank(&candidate.outcome.proof) < proof_rank(&existing.outcome.proof) {
                    *existing = candidate;
                }
            }
            None => options.push(candidate),
        }
    }
    options.sort_by_key(|o| proof_rank(&o.outcome.proof));
    options.truncate(MAX_OPTIONS);

    match options.len() {
        0 => Resolution::NotFound { queries },
        1 => Resolution::Resolved(options.remove(0).outcome),
        _ => Resolution::Ambiguous(
            options
                .into_iter()
                .map(
                    |Candidate {
                         outcome,
                         mut evidence,
                         ..
                     }| {
                        evidence.scrobbled_group =
                            outcome.scrobbled_group.as_ref().map(ReleaseGroup::describe);
                        evidence.canonical_group =
                            outcome.canonical.as_ref().map(|c| c.group.describe());
                        (outcome, evidence)
                    },
                )
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::outcome::{derive_edit, Approval, Chosen, MovePolicy, Naming};
    use crate::identity::testing::*;
    use crate::subject::Subject;

    fn key(artist: &str, track: &str, album: Option<&str>) -> ScrobbleKey {
        ScrobbleKey {
            artist: artist.into(),
            track: track.into(),
            album: album.map(str::to_string),
        }
    }

    async fn resolve(mb: &FixtureMb, key: &ScrobbleKey) -> Resolution {
        Resolver {
            mb,
            limits: ResolveLimits::default(),
        }
        .resolve(key)
        .await
        .unwrap()
    }

    fn plain(group: ReleaseGroup) -> Option<Canonical> {
        Some(Canonical {
            group,
            rung: Rung::PlainAlbum,
            single_first: false,
        })
    }

    #[tokio::test]
    async fn compilation_track_resolves_to_studio_album_through_shared_isrc() {
        let mb = FixtureMb::default().cocaine();
        let resolution = resolve(
            &mb,
            &key("Eric Clapton", "Cocaine", Some("The Cream of Clapton")),
        )
        .await;
        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
                title: "Cocaine".into(),
                scrobbled_group: Some(cream_of_clapton()),
                canonical: plain(slowhand()),
                proof: Proof::SharedIsrc {
                    isrc: "NLF057790024".into(),
                    len_delta_ms: 2_107,
                },
            })
        );
    }

    #[tokio::test]
    async fn plain_album_edition_takes_one_search_and_no_browse() {
        let mb = FixtureMb::default().cocaine();
        let resolution = resolve(
            &mb,
            &key(
                "Eric Clapton",
                "Cocaine",
                Some("Slowhand 35th Anniversary (Super Deluxe)"),
            ),
        )
        .await;
        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid(COCAINE_51)],
                title: "Cocaine".into(),
                scrobbled_group: Some(slowhand()),
                canonical: plain(slowhand()),
                proof: Proof::SameRecording,
            })
        );
        assert_eq!(mb.calls(), vec!["search Cocaine | Eric Clapton | Slowhand"]);
    }

    #[tokio::test]
    async fn remaster_suffix_resolves_to_the_plain_mb_title() {
        let mb = FixtureMb::default().wild_horses();
        let resolution = resolve(
            &mb,
            &key(
                "The Rolling Stones",
                "Wild Horses - 2009 Re-Mastered Digital Version",
                Some("Sticky Fingers"),
            ),
        )
        .await;
        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid("5a6062a4-c790-4c29-84c3-080ea9e332ef")],
                title: "Wild Horses".into(),
                scrobbled_group: Some(sticky_fingers()),
                canonical: plain(sticky_fingers()),
                proof: Proof::SameRecording,
            })
        );
        assert_eq!(
            mb.calls(),
            vec!["search Wild Horses | The Rolling Stones | Sticky Fingers"]
        );
    }

    #[tokio::test]
    async fn live_album_track_stays_on_the_live_album() {
        let mb = FixtureMb::default().four_on_six();
        let resolution = resolve(
            &mb,
            &key(
                "Wes Montgomery",
                "Four on Six",
                Some("Smokin' at the Half Note"),
            ),
        )
        .await;
        let Resolution::Resolved(outcome) = &resolution else {
            panic!("expected a resolution, got {resolution:?}");
        };
        assert_eq!(
            outcome,
            &Outcome {
                master: vec![rid(FOUR_ON_SIX_LIVE)],
                title: "Four on Six".into(),
                scrobbled_group: Some(smokin()),
                canonical: None,
                proof: Proof::SameRecording,
            }
        );
    }

    #[tokio::test]
    async fn candidates_implying_different_edits_are_ambiguous() {
        let mb = FixtureMb::default().wild_horses_on_honk();
        let resolution =
            resolve(&mb, &key("The Rolling Stones", "Wild Horses", Some("Honk"))).await;
        assert_eq!(
            resolution,
            Resolution::Ambiguous(vec![
                (
                    Outcome {
                        master: vec![rid(WILD_HORSES_2009)],
                        title: "Wild Horses".into(),
                        scrobbled_group: Some(honk()),
                        canonical: plain(sticky_fingers()),
                        proof: Proof::SameRecording,
                    },
                    OptionEvidence {
                        recordings: vec![(
                            WILD_HORSES_2009.into(),
                            "Wild Horses".into(),
                            "2009 mix".into(),
                            Some(343_000)
                        )],
                        isrcs: vec!["GBUM70909466".into()],
                        work_attributes: vec![],
                        scrobbled_group: Some("Honk (Album + Compilation, 2019)".into()),
                        canonical_group: Some("Sticky Fingers (Album, 1971)".into()),
                    },
                ),
                (
                    Outcome {
                        master: vec![rid(WILD_HORSES_COMP)],
                        title: "Wild Horses".into(),
                        scrobbled_group: Some(honk()),
                        canonical: None,
                        proof: Proof::SameRecording,
                    },
                    OptionEvidence {
                        recordings: vec![(
                            WILD_HORSES_COMP.into(),
                            "Wild Horses".into(),
                            "".into(),
                            Some(342_000)
                        )],
                        isrcs: vec![],
                        work_attributes: vec![],
                        scrobbled_group: Some("Honk (Album + Compilation, 2019)".into()),
                        canonical_group: None,
                    },
                ),
            ])
        );
    }

    #[tokio::test]
    async fn isrc_siblings_must_share_the_title_and_an_artist() {
        let ocean_boulevard = group(
            "461-ocean-boulevard",
            "461 Ocean Boulevard",
            PrimaryType::Album,
            &[],
            1974,
            clapton(),
        );
        let jj_cale = || credit(&[("jj-cale", "J.J. Cale", "")]);
        let mb = FixtureMb::default()
            .cocaine()
            .with_isrc(
                "NLF057790024",
                &[
                    COCAINE_CREAM,
                    COCAINE_STUDIO,
                    "i-shot-the-sheriff",
                    "cale-cocaine",
                ],
            )
            .with_recording(detail(
                "i-shot-the-sheriff",
                "I Shot the Sheriff",
                "",
                218_000,
                clapton(),
                &["NLF057790024"],
                vec![],
            ))
            .with_recording(detail(
                "cale-cocaine",
                "Cocaine",
                "",
                219_000,
                jj_cale(),
                &["NLF057790024"],
                vec![],
            ))
            .with_releases("i-shot-the-sheriff", vec![rel(&ocean_boulevard)])
            .with_releases(
                "cale-cocaine",
                vec![rel(&group(
                    "troubadour",
                    "Troubadour",
                    PrimaryType::Album,
                    &[],
                    1976,
                    jj_cale(),
                ))],
            );
        let resolution = resolve(
            &mb,
            &key("Eric Clapton", "Cocaine", Some("The Cream of Clapton")),
        )
        .await;
        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
                title: "Cocaine".into(),
                scrobbled_group: Some(cream_of_clapton()),
                canonical: plain(slowhand()),
                proof: Proof::SharedIsrc {
                    isrc: "NLF057790024".into(),
                    len_delta_ms: 2_107,
                },
            })
        );
    }

    #[tokio::test]
    async fn isrc_siblings_must_be_the_same_version() {
        let beatles = || credit(&[("b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d", "The Beatles", "")]);
        let one = group(
            "one",
            "1",
            PrimaryType::Album,
            &[SecondaryType::Compilation],
            2015,
            beatles(),
        );
        let magical_mystery_tour = group(
            "magical-mystery-tour",
            "Magical Mystery Tour",
            PrimaryType::Album,
            &[],
            1967,
            beatles(),
        );
        let remix = "strawberry-fields-2015-remix";
        let original = "strawberry-fields-original";
        // (how MB names the remix: title, disambiguation)
        let cases = [
            ("Strawberry Fields Forever (2015 Remix)", ""),
            ("Strawberry Fields Forever", "2015 remix"),
        ];
        for (title, disambiguation) in cases {
            let mb = FixtureMb::default()
                .with_search(
                    "Strawberry Fields Forever",
                    "The Beatles",
                    Some("1"),
                    vec![hit(
                        remix,
                        100,
                        title,
                        disambiguation,
                        247_000,
                        beatles(),
                        vec![rel(&one)],
                    )],
                )
                .with_recording(detail(
                    remix,
                    title,
                    disambiguation,
                    247_000,
                    beatles(),
                    &["GBUM71505902"],
                    vec![],
                ))
                .with_recording(detail(
                    original,
                    "Strawberry Fields Forever",
                    "",
                    248_000,
                    beatles(),
                    &["GBUM71505902"],
                    vec![],
                ))
                .with_isrc("GBUM71505902", &[remix, original])
                .with_releases(remix, vec![rel(&one)])
                .with_releases(original, vec![rel(&magical_mystery_tour)]);

            let resolution = resolve(
                &mb,
                &key(
                    "The Beatles",
                    "Strawberry Fields Forever - 2015 Remix",
                    Some("1"),
                ),
            )
            .await;

            assert_eq!(
                resolution,
                Resolution::Resolved(Outcome {
                    master: vec![rid(remix)],
                    title: title.into(),
                    scrobbled_group: Some(one.clone()),
                    canonical: None,
                    proof: Proof::SameRecording,
                }),
                "{title} ({disambiguation})"
            );
        }
    }

    #[tokio::test]
    async fn a_clean_scrobble_matches_only_the_clean_recording() {
        let outkast = || credit(&[("73fdb566-a9b1-494c-9f32-51768ec9fd56", "OutKast", "")]);
        let speakerboxxx = group(
            "speakerboxxx",
            "Speakerboxxx/The Love Below",
            PrimaryType::Album,
            &[],
            2003,
            outkast(),
        );
        let explicit = "hey-ya-explicit";
        let clean = "hey-ya-clean";
        let take = |id: &str, disambiguation: &str| {
            hit(
                id,
                100,
                "Hey Ya!",
                disambiguation,
                235_000,
                outkast(),
                vec![rel(&speakerboxxx)],
            )
        };
        let mb = FixtureMb::default().with_search(
            "Hey Ya!",
            "OutKast",
            Some("Speakerboxxx/The Love Below"),
            vec![take(explicit, ""), take(clean, "clean")],
        );
        let scrobble = key(
            "OutKast",
            "Hey Ya! (Clean)",
            Some("Speakerboxxx/The Love Below"),
        );

        let resolution = resolve(&mb, &scrobble).await;

        let expected = Outcome {
            master: vec![rid(clean)],
            title: "Hey Ya!".into(),
            scrobbled_group: Some(speakerboxxx.clone()),
            canonical: plain(speakerboxxx),
            proof: Proof::SameRecording,
        };
        assert_eq!(resolution, Resolution::Resolved(expected.clone()));
        let subject = Subject {
            artist: scrobble.artist,
            track: scrobble.track,
            album: scrobble.album,
            album_artist: None,
        };
        assert_eq!(
            derive_edit(
                &subject,
                &expected,
                Chosen::Deterministic,
                &[],
                &MovePolicy::default(),
                Naming::reviewer(),
            ),
            None,
            "\"(Clean)\" names the version and is never stripped"
        );
    }

    #[tokio::test]
    async fn a_truncated_search_never_proves_the_match() {
        let mut mb = FixtureMb::default().cocaine();
        mb.truncated_searches.insert(query(
            "Cocaine",
            "Eric Clapton",
            Some("The Cream of Clapton"),
        ));
        let scrobble = key("Eric Clapton", "Cocaine", Some("The Cream of Clapton"));

        let resolution = resolve(&mb, &scrobble).await;

        let expected = Outcome {
            master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
            title: "Cocaine".into(),
            scrobbled_group: Some(cream_of_clapton()),
            canonical: plain(slowhand()),
            proof: Proof::TitleOnly,
        };
        assert_eq!(resolution, Resolution::Resolved(expected.clone()));
        let subject = Subject {
            artist: scrobble.artist,
            track: scrobble.track,
            album: scrobble.album,
            album_artist: None,
        };
        let proposal = derive_edit(
            &subject,
            &expected,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .expect("the move is still proposed");
        assert_eq!(
            (proposal.target.album.as_deref(), proposal.approval),
            (Some("Slowhand"), Approval::Review)
        );
    }

    #[test]
    fn single_suffix_only_matches_a_single_or_ep_group() {
        let prince = || credit(&[("prince", "Prince", "")]);
        let album = group(
            "purple-rain-album",
            "Purple Rain",
            PrimaryType::Album,
            &[],
            1984,
            prince(),
        );
        let single = group(
            "purple-rain-single",
            "Purple Rain",
            PrimaryType::Single,
            &[],
            1984,
            prince(),
        );
        let title =
            |releases: &[ReleaseRef], album: &str| scrobbled_group(releases, album).map(|g| g.id.0);
        let both = [rel(&album), rel(&single)];
        assert_eq!(title(&[rel(&album)], "Purple Rain - Single"), None);
        assert_eq!(
            title(&both, "Purple Rain - Single"),
            Some("purple-rain-single".into())
        );
        assert_eq!(
            title(&both, "Purple Rain (Deluxe Edition)"),
            Some("purple-rain-album".into())
        );
        assert_eq!(
            title(&both, "Purple Rain"),
            Some("purple-rain-album".into())
        );
    }

    #[tokio::test]
    async fn compilation_track_without_isrc_link_finds_the_studio_take_by_work() {
        let mut mb = FixtureMb::default().cocaine();
        mb.recordings
            .get_mut(&rid(COCAINE_CREAM))
            .unwrap()
            .isrcs
            .clear();
        let resolution = resolve(
            &mb,
            &key("Eric Clapton", "Cocaine", Some("The Cream of Clapton")),
        )
        .await;
        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
                title: "Cocaine".into(),
                scrobbled_group: Some(cream_of_clapton()),
                canonical: plain(slowhand()),
                proof: Proof::SameWork,
            })
        );
    }

    #[tokio::test]
    async fn truncated_browse_offers_staying_put() {
        let mut mb = FixtureMb::default().cocaine();
        mb.releases
            .get_mut(&rid(COCAINE_STUDIO))
            .unwrap()
            .push(rel(&cream_of_clapton()));
        let limits = ResolveLimits {
            max_releases: 3,
            ..ResolveLimits::default()
        };
        let resolution = Resolver { mb: &mb, limits }
            .resolve(&key(
                "Eric Clapton",
                "Cocaine",
                Some("The Cream of Clapton"),
            ))
            .await
            .unwrap();
        let Resolution::Ambiguous(options) = resolution else {
            panic!("expected ambiguity, got {resolution:?}");
        };
        let canonicals: Vec<_> = options
            .iter()
            .map(|(outcome, _)| outcome.canonical.as_ref().map(|c| c.group.title.as_str()))
            .collect();
        assert_eq!(canonicals, vec![None, Some("Slowhand")]);
    }

    #[tokio::test]
    async fn untitled_album_falls_back_to_title_only() {
        let mb = FixtureMb::default().with_search(
            "Cocaine",
            "Eric Clapton",
            None,
            vec![hit(
                COCAINE_STUDIO,
                100,
                "Cocaine",
                "original studio mix",
                216_640,
                clapton(),
                vec![],
            )],
        );
        let resolution = resolve(&mb, &key("Eric Clapton", "Cocaine", None)).await;
        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid(COCAINE_STUDIO)],
                title: "Cocaine".into(),
                scrobbled_group: None,
                canonical: None,
                proof: Proof::TitleOnly,
            })
        );
    }

    #[tokio::test]
    async fn empty_searches_walk_the_whole_ladder() {
        let mb = FixtureMb::default();
        let resolution = resolve(
            &mb,
            &key(
                "The Rolling Stones",
                "Wild Horses - 2009 Re-Mastered Digital Version",
                Some("Sticky Fingers (Deluxe Edition)"),
            ),
        )
        .await;
        assert_eq!(
            mb.calls(),
            vec![
                "search Wild Horses | The Rolling Stones | Sticky Fingers",
                "search Wild Horses | The Rolling Stones",
                "search Wild Horses - 2009 Re-Mastered Digital Version | The Rolling Stones",
            ]
        );
        assert_eq!(
            resolution,
            Resolution::NotFound {
                queries: vec![
                    query("Wild Horses", "The Rolling Stones", Some("Sticky Fingers")).lucene(),
                    query("Wild Horses", "The Rolling Stones", None).lucene(),
                    query(
                        "Wild Horses - 2009 Re-Mastered Digital Version",
                        "The Rolling Stones",
                        None
                    )
                    .lucene(),
                ]
            }
        );
    }

    #[tokio::test]
    async fn credited_remix_only_matches_the_same_remixer() {
        let jack_u = || credit(&[("0eb2a6ef-5aa5-4b8f-8d2e-4dc9d0d3e3b1", "Jack Ü", "")]);
        let remix = |id: &str, title: &str| hit(id, 100, title, "", 200_000, jack_u(), vec![]);
        let mb = FixtureMb::default().with_search(
            "To Ü",
            "Jack Ü",
            None,
            vec![
                remix(
                    "11111111-1111-1111-1111-111111111111",
                    "To Ü (Manikin remix)",
                ),
                remix(
                    "22222222-2222-2222-2222-222222222222",
                    "To Ü (Oliver remix)",
                ),
            ],
        );

        let resolution = resolve(&mb, &key("Jack Ü", "To Ü - Oliver Remix", None)).await;

        let Resolution::Resolved(outcome) = resolution else {
            panic!("expected a single outcome, got {resolution:?}");
        };
        assert_eq!(outcome.title, "To Ü (Oliver remix)");
        assert_eq!(
            outcome.master,
            vec![rid("22222222-2222-2222-2222-222222222222")]
        );
    }

    #[tokio::test]
    async fn typographic_title_variants_are_one_outcome() {
        let dylan = || credit(&[("72c536dc-4b47-4f12-8d8a-1d4b8e3a5d2b", "Bob Dylan", "")]);
        let freewheelin = group(
            "c0cbd0b3-5b7e-3d1b-a8e3-1a5f4b6f1d0e",
            "The Freewheelin’ Bob Dylan",
            PrimaryType::Album,
            &[],
            1963,
            dylan(),
        );
        let take = |id: &str, title: &str| {
            hit(
                id,
                100,
                title,
                "",
                220_000,
                dylan(),
                vec![rel(&freewheelin)],
            )
        };
        let mb = FixtureMb::default().with_search(
            "Don't Think Twice, It's All Right",
            "Bob Dylan",
            Some("The Freewheelin' Bob Dylan"),
            vec![
                take(
                    "33333333-3333-3333-3333-333333333333",
                    "Don't Think Twice, It's All Right",
                ),
                take(
                    "44444444-4444-4444-4444-444444444444",
                    "Don’t Think Twice, It’s All Right",
                ),
            ],
        );

        let resolution = resolve(
            &mb,
            &key(
                "Bob Dylan",
                "Don't Think Twice, It's All Right",
                Some("The Freewheelin' Bob Dylan"),
            ),
        )
        .await;

        assert!(
            matches!(resolution, Resolution::Resolved(_)),
            "expected one outcome, got {resolution:?}"
        );
    }

    fn waltz_for_debby() -> ScrobbleKey {
        key("Bill Evans", "Waltz for Debby", Some("Waltz For Debby"))
    }

    #[tokio::test]
    async fn a_trio_credit_matches_its_leader() {
        let mb = FixtureMb::default().waltz_for_debby().with_search(
            "Waltz for Debby",
            "Bill Evans",
            Some("Waltz For Debby"),
            vec![hit(
                WALTZ_FOR_DEBBY_TAKE_2,
                100,
                "Waltz for Debby",
                "take 2 - master",
                420_000,
                evans_trio_with_lafaro_and_motian(),
                vec![rel(&waltz_for_debby_live())],
            )],
        );

        let resolution = resolve(&mb, &waltz_for_debby()).await;

        let Resolution::Resolved(outcome) = resolution else {
            panic!("expected a resolution, got {resolution:?}");
        };
        assert_eq!(outcome.scrobbled_group, Some(waltz_for_debby_live()));
    }

    #[test]
    fn ensemble_credits_match_only_their_leader_by_whole_words() {
        let one = |name: &str| credit(&[("id", name, "")]);
        let cases = [
            ("Bill Evans", "Bill Evans Trio", true),
            ("Dave Brubeck", "The Dave Brubeck Quartet", true),
            ("Bob Marley", "Bob Marley & The Wailers", true),
            ("Count Basie", "Count Basie Orchestra", true),
            ("Steve Miller", "Steve Miller Bandit", false),
            ("Bill", "Bill Evans Trio", false),
            ("Bill Evans", "Bill Evans Quintet Revisited", true),
            ("Bill Evans", "Tony Bennett & Bill Evans", false),
        ];
        for (artist, credited, expected) in cases {
            assert_eq!(
                artist_matches(artist, &one(credited)),
                expected,
                "{artist} vs {credited}"
            );
        }
    }

    async fn verify(
        mb: &FixtureMb,
        recording: &str,
        release_group: Option<&str>,
    ) -> Option<Outcome> {
        Resolver {
            mb,
            limits: ResolveLimits::default(),
        }
        .verify_research(
            &waltz_for_debby(),
            &rid(recording),
            release_group.map(|id| ReleaseGroupId(id.into())).as_ref(),
            "claude:opus",
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn verified_research_settles_waltz_for_debby_on_the_live_album_without_an_edit() {
        let mb = FixtureMb::default().waltz_for_debby();

        let outcome = verify(
            &mb,
            WALTZ_FOR_DEBBY_TAKE_2,
            Some("392e7fcf-5ae9-37f6-b8c9-57901cfbee1f"),
        )
        .await;

        let expected = Outcome {
            master: vec![rid(WALTZ_FOR_DEBBY_TAKE_2)],
            title: "Waltz for Debby".into(),
            scrobbled_group: Some(waltz_for_debby_live()),
            canonical: None,
            proof: Proof::Researched {
                by: "claude:opus".into(),
            },
        };
        assert_eq!(outcome, Some(expected.clone()));
        let subject = Subject {
            artist: "Bill Evans".into(),
            track: "Waltz for Debby".into(),
            album: Some("Waltz For Debby".into()),
            album_artist: None,
        };
        assert_eq!(
            derive_edit(
                &subject,
                &expected,
                Chosen::Adjudicated,
                &[],
                &MovePolicy::default(),
                Naming::reviewer(),
            ),
            None
        );
    }

    #[tokio::test]
    async fn research_answers_that_are_not_this_scrobble_fail_verification() {
        let my_foolish_heart = "8a3a8a4e-2c39-4d2f-9f0b-3e3c0c7c1a11";
        let zetterlund = "0f4e2b8b-6a4c-4b8e-8f6a-2a3b8d6c9e21";
        let mb = FixtureMb::default()
            .waltz_for_debby()
            .with_recording(detail(
                my_foolish_heart,
                "My Foolish Heart",
                "",
                296_000,
                evans_trio_with_lafaro_and_motian(),
                &[],
                vec![],
            ))
            .with_releases(my_foolish_heart, vec![rel(&waltz_for_debby_live())])
            .with_recording(detail(
                zetterlund,
                "Waltz for Debby",
                "",
                165_000,
                credit(&[("monica-zetterlund", "Monica Zetterlund", "")]),
                &[],
                vec![],
            ));
        let cases = [
            (
                "unknown to MusicBrainz",
                "1d0b6fbe-3f4e-4e43-9b52-0d3a2c1e5f77",
            ),
            ("another song on the album", my_foolish_heart),
            ("another artist's recording", zetterlund),
        ];
        for (name, recording) in cases {
            assert_eq!(
                verify(&mb, recording, Some("392e7fcf-5ae9-37f6-b8c9-57901cfbee1f")).await,
                None,
                "{name}"
            );
        }
    }
}
