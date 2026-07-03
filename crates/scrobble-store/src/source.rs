//! The remote replica's read path, abstracted so the sync engine never deals with
//! source-specific pagination quirks — and so tests can script arbitrary timelines.

use crate::error::Result;
use crate::record::RecordSource;
use lastfm_edit::{RateLimitStateWatcher, Track};

/// One fetched page of scrobbles for a window.
#[derive(Debug, Clone)]
pub struct SourcePage {
    /// Scrobbles with `timestamp < to` (the exclusive window pin). Sources must *not*
    /// filter out tracks older than `from` — the engine uses the first track past that
    /// boundary as its "fetched past the window" completion signal. Order within the page
    /// is not assumed.
    pub tracks: Vec<Track>,
    /// Whether the source reports more pages after this one.
    pub has_next: bool,
}

/// A paginated, time-windowable view of the upstream scrobble timeline.
///
/// Implementations must guarantee that, for a fixed window `(from, to)`, paging from 1
/// upward visits every scrobble with `from <= timestamp < to` at least once (duplicates
/// across pages are fine — the store deduplicates). New scrobbles arriving *above* `to`
/// while paging must not affect which older scrobbles are visited; this is what makes a
/// `to`-pinned pass deterministic.
#[async_trait::async_trait(?Send)]
pub trait ScrobbleSource {
    /// Which [`RecordSource`] tag records fetched through this source carry (this drives
    /// album-artist provenance).
    fn record_source(&self) -> RecordSource;

    /// Fetch page `page` (1-indexed) of scrobbles within the window.
    async fn fetch_window(
        &self,
        from: Option<u64>,
        to: Option<u64>,
        page: u32,
    ) -> Result<SourcePage>;

    /// Watch the underlying client's rate-limit state, so the engine can pause and report
    /// instead of hammering a parked client.
    fn rate_limit(&self) -> RateLimitStateWatcher;
}

/// [`ScrobbleSource`] backed by the official Last.fm JSON API (200 scrobbles/page, much
/// friendlier rate limits, server-side time windows — the preferred bulk source).
/// Records from here carry `RecordSource::Api`, i.e. album artist is never trusted.
pub struct ApiSource {
    client: lastfm_edit::LastFmApiClientImpl,
}

impl ApiSource {
    pub fn new(client: lastfm_edit::LastFmApiClientImpl) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait(?Send)]
impl ScrobbleSource for ApiSource {
    fn record_source(&self) -> RecordSource {
        RecordSource::Api
    }

    async fn fetch_window(
        &self,
        from: Option<u64>,
        to: Option<u64>,
        page: u32,
    ) -> Result<SourcePage> {
        use lastfm_edit::LastFmApiClient;
        // Observed live (lastfm-edit's `api_recent_tracks_in_range` VCR test): the API
        // window is natively half-open [from, to). The one-second widening of `from` is
        // deliberate belt-and-braces against a server-side behavior change — the overlap
        // is absorbed by the store's dedup — and the `ts < pin` clip re-asserts the pin
        // locally rather than trusting the server.
        let api_from = from.map(|f| f.saturating_sub(1));
        let page = self
            .client
            .api_get_recent_tracks_page_in_range(page, api_from, to)
            .await?;
        let tracks = match to {
            Some(pin) => page
                .tracks
                .into_iter()
                .filter(|t| t.timestamp.is_none_or(|ts| ts < pin))
                .collect(),
            None => page.tracks,
        };
        Ok(SourcePage {
            tracks,
            has_next: page.has_next_page,
        })
    }

    fn rate_limit(&self) -> RateLimitStateWatcher {
        self.client.watch_rate_limit_state()
    }
}

/// [`ScrobbleSource`] backed by scraping the Last.fm library pages via a
/// [`LastFmBaseClient`](lastfm_edit::LastFmBaseClient).
///
/// The library listing has no server-side time filter, so windows are emulated by paging
/// from the newest scrobble and clipping to the pin. Correct for any window, but
/// **inefficient for windows deep in the past** (it must page through everything newer
/// first) — prefer [`ApiSource`] for backfills and use this when no API key is available.
/// Records carry `RecordSource::Scrape`, so album artists parsed from pages are trusted.
pub struct ScrapeSource<C> {
    client: C,
}

impl<C> ScrapeSource<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait(?Send)]
impl<C: lastfm_edit::LastFmBaseClient> ScrobbleSource for ScrapeSource<C> {
    fn record_source(&self) -> RecordSource {
        RecordSource::Scrape
    }

    async fn fetch_window(
        &self,
        _from: Option<u64>,
        to: Option<u64>,
        page: u32,
    ) -> Result<SourcePage> {
        let page = self.client.get_recent_tracks_page(page).await?;
        let tracks = match to {
            Some(pin) => page
                .tracks
                .into_iter()
                .filter(|t| t.timestamp.is_none_or(|ts| ts < pin))
                .collect(),
            None => page.tracks,
        };
        Ok(SourcePage {
            tracks,
            has_next: page.has_next_page,
        })
    }

    fn rate_limit(&self) -> RateLimitStateWatcher {
        self.client.watch_rate_limit_state()
    }
}
