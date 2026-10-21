//! The paced, cached HTTP implementation of [`MbLookup`]. Wire types stay private here.

use super::cache::MbCache;
use super::{
    ArtistCredit, ArtistId, CreditPart, Isrc, MbError, MbLookup, PartialDate, PrimaryType,
    RecordingDetail, RecordingHit, RecordingId, RecordingQuery, ReleaseGroup, ReleaseGroupId,
    ReleaseRef, SecondaryType, Truncated, UserAgent, WorkId, WorkLink,
};
use http_client::http_types::{Method, Request, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

pub(crate) const BASE_URL: &str = "https://musicbrainz.org/ws/2/";

const MIN_INTERVAL: Duration = Duration::from_millis(1100);
const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Longer server waits defer the key instead of stalling the lane.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(300);
const MAX_REDIRECTS: usize = 3;
const SEARCH_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const LOOKUP_TTL: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const SEARCH_LIMIT: &str = "100";
const PAGE_LIMIT: &str = "100";

/// One per process (owned by the identity lane), so the pacer is not shared state.
pub(crate) struct MusicBrainz {
    http: Arc<dyn http_client::HttpClient>,
    user_agent: UserAgent,
    pacer: Pacer,
    cache: MbCache,
    stats: std::sync::Mutex<MbStats>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MbStats {
    pub requests: u64,
    pub cache_hits: u64,
}

impl MusicBrainz {
    pub fn new(
        http: Arc<dyn http_client::HttpClient>,
        user_agent: UserAgent,
        cache_path: &Path,
    ) -> Result<Self, MbError> {
        Ok(Self {
            http,
            user_agent,
            pacer: Pacer::new(MIN_INTERVAL),
            cache: MbCache::open(cache_path)?,
            stats: std::sync::Mutex::new(MbStats::default()),
        })
    }

    pub fn stats(&self) -> MbStats {
        *self.stats.lock().expect("stats lock")
    }

    /// Cache-first: a fresh hit never reaches the pacer.
    async fn cached<W, T>(
        &self,
        key: &str,
        ttl: Duration,
        path: &str,
        query: &[(&str, &str)],
        convert: impl FnOnce(W) -> T,
    ) -> Result<T, MbError>
    where
        W: DeserializeOwned,
        T: Serialize + DeserializeOwned,
    {
        if let Some(hit) = self.cache.get(key, ttl)? {
            self.stats.lock().expect("stats lock").cache_hits += 1;
            return Ok(hit);
        }
        let value = convert(self.get_json(path, query).await?);
        self.cache.put(key, &value)?;
        Ok(value)
    }

    /// Follows MB's paging until `cap` rows or the reported count. MB may return fewer
    /// rows than the limit, so the offset advances by rows actually returned.
    async fn paged<W, T>(
        &self,
        key_prefix: &str,
        path: &str,
        query: &[(&str, &str)],
        cap: usize,
        convert: impl Fn(W) -> Page<T>,
    ) -> Result<(Vec<T>, Truncated), MbError>
    where
        W: DeserializeOwned,
        T: Serialize + DeserializeOwned,
    {
        let mut rows = Vec::new();
        let mut offset = 0;
        loop {
            let offset_param = offset.to_string();
            let mut page_query = query.to_vec();
            page_query.extend([("limit", PAGE_LIMIT), ("offset", &offset_param)]);
            let page = self
                .cached(
                    &format!("{key_prefix}:{offset}"),
                    LOOKUP_TTL,
                    path,
                    &page_query,
                    &convert,
                )
                .await?;
            let returned = page.rows.len();
            rows.extend(page.rows);
            offset += returned;
            if returned == 0 || offset >= page.count || rows.len() >= cap {
                let truncated = rows.len() > cap || offset < page.count;
                rows.truncate(cap);
                return Ok((rows, Truncated(truncated)));
            }
        }
    }

    /// One paced GET with `fmt=json`.
    async fn get_json<W: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<W, MbError> {
        let mut url = Url::parse(BASE_URL)
            .and_then(|base| base.join(path))
            .map_err(|e| MbError::Transport(format!("{path}: {e}")))?;
        url.query_pairs_mut().extend_pairs(query);
        for _ in 0..=MAX_REDIRECTS {
            if !url.query_pairs().any(|(name, _)| name == "fmt") {
                url.query_pairs_mut().append_pair("fmt", "json");
            }
            self.pacer.acquire().await;
            self.stats.lock().expect("stats lock").requests += 1;
            let mut request = Request::new(Method::Get, url.clone());
            request
                .insert_header("User-Agent", self.user_agent.as_str())
                .and_then(|_| request.insert_header("Accept", "application/json"))
                .map_err(|e| MbError::Transport(format!("{url}: {e}")))?;
            let mut response = self
                .http
                .send(request)
                .await
                .map_err(|e| MbError::Transport(format!("{url}: {e}")))?;
            match response.status() {
                status if status.is_success() => {
                    self.pacer.clear_backoff().await;
                    let body = response
                        .body_string()
                        .await
                        .map_err(|e| MbError::Transport(format!("{url}: {e}")))?;
                    return serde_json::from_str(&body)
                        .map_err(|e| MbError::Parse(format!("{url}: {e}")));
                }
                StatusCode::MovedPermanently
                | StatusCode::Found
                | StatusCode::TemporaryRedirect
                | StatusCode::PermanentRedirect => {
                    let location = response
                        .header("Location")
                        .map(|values| values.last().as_str().to_string())
                        .ok_or_else(|| {
                            MbError::Parse(format!("{url}: redirect without Location"))
                        })?;
                    url = url.join(&location).map_err(|e| {
                        MbError::Parse(format!("{url}: bad Location {location}: {e}"))
                    })?;
                }
                StatusCode::NotFound => return Err(MbError::NotFound),
                StatusCode::TooManyRequests | StatusCode::ServiceUnavailable => {
                    let retry_after = response
                        .header("Retry-After")
                        .and_then(|values| values.last().as_str().trim().parse().ok())
                        .map(Duration::from_secs);
                    return Err(MbError::RateLimited(self.pacer.penalize(retry_after).await));
                }
                status => return Err(MbError::Transport(format!("{url}: HTTP {status}"))),
            }
        }
        Err(MbError::Transport(format!("{url}: too many redirects")))
    }
}

#[async_trait::async_trait]
impl MbLookup for MusicBrainz {
    async fn search_recordings(
        &self,
        query: &RecordingQuery,
    ) -> Result<(Vec<RecordingHit>, Truncated), MbError> {
        let lucene = query.lucene();
        self.cached(
            &format!("search:{lucene}"),
            SEARCH_TTL,
            "recording",
            &[("query", &lucene), ("limit", SEARCH_LIMIT)],
            |wire: WireSearch| {
                let hits: Vec<RecordingHit> = wire
                    .recordings
                    .into_iter()
                    .map(WireRecording::into_hit)
                    .collect();
                let truncated = Truncated(wire.count > hits.len());
                (hits, truncated)
            },
        )
        .await
    }

    async fn recording(&self, id: &RecordingId) -> Result<RecordingDetail, MbError> {
        let RecordingId(id) = id;
        self.cached(
            &format!("recording:{id}"),
            LOOKUP_TTL,
            &format!("recording/{id}"),
            &[("inc", "isrcs work-rels artist-credits")],
            WireRecording::into_detail,
        )
        .await
    }

    async fn isrc_members(&self, isrc: &Isrc) -> Result<Vec<RecordingId>, MbError> {
        let Isrc(code) = isrc;
        self.cached(
            &format!("isrc:{code}"),
            LOOKUP_TTL,
            &format!("isrc/{code}"),
            &[],
            |wire: WireIsrc| {
                wire.recordings
                    .into_iter()
                    .map(|r| RecordingId(r.id))
                    .collect()
            },
        )
        .await
    }

    async fn work_recordings(
        &self,
        work: &WorkId,
        cap: usize,
    ) -> Result<(Vec<RecordingId>, Truncated), MbError> {
        let WorkId(id) = work;
        self.paged(
            &format!("work-recordings:{id}"),
            "recording",
            &[("work", id.as_str())],
            cap,
            |wire: WireRecordingPage| Page {
                count: wire.recording_count,
                rows: wire
                    .recordings
                    .into_iter()
                    .map(|r| RecordingId(r.id))
                    .collect(),
            },
        )
        .await
    }

    async fn recording_releases(
        &self,
        id: &RecordingId,
        cap: usize,
    ) -> Result<(Vec<ReleaseRef>, Truncated), MbError> {
        let RecordingId(id) = id;
        self.paged(
            &format!("recording-releases:{id}"),
            "release",
            &[
                ("recording", id.as_str()),
                ("status", "official"),
                ("inc", "release-groups artist-credits"),
            ],
            cap,
            |wire: WireReleasePage| Page {
                count: wire.release_count,
                rows: wire
                    .releases
                    .into_iter()
                    .map(|release| release.into_ref(&ArtistCredit::default()))
                    .collect(),
            },
        )
        .await
    }
}

/// Minimum spacing between requests, pushed out by server backoff.
pub(crate) struct Pacer {
    min_interval: Duration,
    state: tokio::sync::Mutex<PacerState>,
}

struct PacerState {
    next_allowed: Instant,
    /// The wait used for the next 503/429 that carries no Retry-After.
    backoff: Duration,
}

impl Pacer {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            state: tokio::sync::Mutex::new(PacerState {
                next_allowed: Instant::now(),
                backoff: INITIAL_BACKOFF,
            }),
        }
    }

    pub async fn acquire(&self) {
        let mut state = self.state.lock().await;
        tokio::time::sleep_until(state.next_allowed).await;
        state.next_allowed = Instant::now() + self.min_interval;
    }

    /// After a 503/429: wait at least `retry_after` (or an exponential default).
    /// Returns the wait applied.
    pub async fn penalize(&self, retry_after: Option<Duration>) -> Duration {
        let mut state = self.state.lock().await;
        let wait = retry_after.filter(|wait| !wait.is_zero()).map_or_else(
            || {
                let wait = state.backoff;
                state.backoff = (wait * 2).min(MAX_BACKOFF);
                wait
            },
            |wait| wait.min(MAX_RETRY_AFTER),
        );
        state.next_allowed = state.next_allowed.max(Instant::now() + wait);
        wait
    }

    async fn clear_backoff(&self) {
        self.state.lock().await.backoff = INITIAL_BACKOFF;
    }
}

#[derive(Serialize, Deserialize)]
struct Page<T> {
    count: usize,
    rows: Vec<T>,
}

fn nullable<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct WireSearch {
    count: usize,
    recordings: Vec<WireRecording>,
}

#[derive(Deserialize)]
struct WireIsrc {
    recordings: Vec<WireId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WireRecordingPage {
    recording_count: usize,
    recordings: Vec<WireId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WireReleasePage {
    release_count: usize,
    releases: Vec<WireRelease>,
}

#[derive(Deserialize)]
struct WireId {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WireRecording {
    id: String,
    #[serde(default)]
    score: Option<u8>,
    title: String,
    #[serde(default, deserialize_with = "nullable")]
    disambiguation: String,
    #[serde(default)]
    length: Option<u32>,
    #[serde(default, deserialize_with = "nullable")]
    artist_credit: Vec<WireCredit>,
    #[serde(default, deserialize_with = "nullable")]
    isrcs: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    relations: Vec<WireRelation>,
    #[serde(default, deserialize_with = "nullable")]
    releases: Vec<WireRelease>,
}

impl WireRecording {
    fn into_hit(self) -> RecordingHit {
        let credit = credit(self.artist_credit);
        RecordingHit {
            id: RecordingId(self.id),
            score: self.score.unwrap_or(0),
            title: self.title,
            disambiguation: self.disambiguation,
            length_ms: self.length,
            snippet_releases: self
                .releases
                .into_iter()
                .map(|release| release.into_ref(&credit))
                .collect(),
            credit,
        }
    }

    fn into_detail(self) -> RecordingDetail {
        RecordingDetail {
            id: RecordingId(self.id),
            title: self.title,
            disambiguation: self.disambiguation,
            length_ms: self.length,
            credit: credit(self.artist_credit),
            isrcs: self.isrcs.into_iter().map(Isrc).collect(),
            works: self
                .relations
                .into_iter()
                .filter(|relation| relation.target_type == "work")
                .filter_map(|relation| {
                    Some(WorkLink {
                        work: WorkId(relation.work?.id),
                        attributes: relation.attributes,
                    })
                })
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WireRelation {
    target_type: String,
    #[serde(default, deserialize_with = "nullable")]
    attributes: Vec<String>,
    #[serde(default)]
    work: Option<WireId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WireRelease {
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    artist_credit: Vec<WireCredit>,
    release_group: WireReleaseGroup,
}

impl WireRelease {
    /// Search snippets omit the group's credit, and the release's too when it equals the
    /// recording's, so the credit falls back group -> release -> `recording_credit`.
    fn into_ref(self, recording_credit: &ArtistCredit) -> ReleaseRef {
        let group = self.release_group;
        let group_credit = [group.artist_credit, self.artist_credit]
            .into_iter()
            .find(|parts| !parts.is_empty())
            .map_or_else(|| recording_credit.clone(), credit);
        ReleaseRef {
            title: self.title,
            official: self.status.as_deref() == Some("Official"),
            date: self.date.as_deref().and_then(PartialDate::parse),
            group: ReleaseGroup {
                id: ReleaseGroupId(group.id),
                title: group.title,
                primary: group.primary_type.as_deref().map(primary_type),
                secondary: group
                    .secondary_types
                    .iter()
                    .map(|s| secondary_type(s))
                    .collect(),
                first_release: group
                    .first_release_date
                    .as_deref()
                    .and_then(PartialDate::parse),
                credit: group_credit,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WireReleaseGroup {
    id: String,
    title: String,
    #[serde(default)]
    primary_type: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    secondary_types: Vec<String>,
    #[serde(default)]
    first_release_date: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    artist_credit: Vec<WireCredit>,
}

#[derive(Deserialize)]
struct WireCredit {
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    joinphrase: String,
    artist: WireId,
}

fn credit(parts: Vec<WireCredit>) -> ArtistCredit {
    ArtistCredit {
        parts: parts
            .into_iter()
            .map(|part| CreditPart {
                artist: ArtistId(part.artist.id),
                name: part.name,
                joinphrase: part.joinphrase,
            })
            .collect(),
    }
}

fn primary_type(name: &str) -> PrimaryType {
    match name {
        "Album" => PrimaryType::Album,
        "Single" => PrimaryType::Single,
        "EP" => PrimaryType::Ep,
        "Broadcast" => PrimaryType::Broadcast,
        _ => PrimaryType::Other,
    }
}

fn secondary_type(name: &str) -> SecondaryType {
    match name {
        "Compilation" => SecondaryType::Compilation,
        "Live" => SecondaryType::Live,
        "Soundtrack" => SecondaryType::Soundtrack,
        "Remix" => SecondaryType::Remix,
        "DJ-mix" => SecondaryType::DjMix,
        "Mixtape/Street" => SecondaryType::Mixtape,
        "Demo" => SecondaryType::Demo,
        "Spokenword" => SecondaryType::Spokenword,
        "Interview" => SecondaryType::Interview,
        "Audiobook" => SecondaryType::Audiobook,
        _ => SecondaryType::Other,
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
