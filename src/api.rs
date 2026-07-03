use crate::iterator::{ApiRecentTracksIterator, AsyncPaginatedIterator};
use crate::types::{
    ClientEvent, ClientEventReceiver, RequestInfo, SharedEventBroadcaster, Track, TrackPage,
};
use crate::Result;
use async_trait::async_trait;
use http_client::{HttpClient, Request};
use http_types::{Method, Url};
use serde::Deserialize;
use std::sync::Arc;

use crate::types::LastFmError;

// =============================================================================
// LastFmApiClient trait and implementation
// =============================================================================

#[async_trait(?Send)]
pub trait LastFmApiClient: Clone {
    /// Fetch a single page of the user's recent tracks via the JSON API.
    ///
    /// Equivalent to [`api_get_recent_tracks_page_in_range`](Self::api_get_recent_tracks_page_in_range)
    /// with no time window.
    async fn api_get_recent_tracks_page(&self, page: u32) -> Result<TrackPage> {
        self.api_get_recent_tracks_page_in_range(page, None, None)
            .await
    }

    /// Fetch a single page of the user's recent tracks restricted to a unix-timestamp window.
    ///
    /// `from` and `to` are passed through to the `user.getRecentTracks` API endpoint's
    /// optional `from`/`to` query parameters. Observed live (see the
    /// `api_recent_tracks_in_range` VCR test): `from` is **inclusive** and `to` is
    /// **exclusive** — a native half-open `[from, to)` window, despite the API docs'
    /// "strictly after"/"strictly before" wording. Callers that must be robust to a
    /// server-side behavior change can widen `from` by one second and dedupe by
    /// timestamp.
    async fn api_get_recent_tracks_page_in_range(
        &self,
        page: u32,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Result<TrackPage>;
}

#[derive(Clone)]
pub struct LastFmApiClientImpl {
    client: Arc<dyn HttpClient + Send + Sync>,
    username: String,
    api_key: String,
    broadcaster: Arc<SharedEventBroadcaster>,
}

impl LastFmApiClientImpl {
    pub fn new(
        client: Box<dyn HttpClient + Send + Sync>,
        username: String,
        api_key: String,
    ) -> Self {
        Self {
            client: Arc::from(client),
            username,
            api_key,
            broadcaster: Arc::new(SharedEventBroadcaster::new()),
        }
    }

    pub fn subscribe(&self) -> ClientEventReceiver {
        self.broadcaster.subscribe()
    }

    pub fn latest_event(&self) -> Option<ClientEvent> {
        self.broadcaster.latest_event()
    }

    /// Get the current rate-limit state snapshot.
    pub fn rate_limit_state(&self) -> crate::types::RateLimitState {
        self.broadcaster.rate_limit_state()
    }

    /// Get a watch receiver tracking rate-limit state transitions.
    pub fn watch_rate_limit_state(&self) -> crate::types::RateLimitStateWatcher {
        self.broadcaster.watch_rate_limit_state()
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn recent_tracks(&self) -> Box<dyn AsyncPaginatedIterator<Track>> {
        Box::new(ApiRecentTracksIterator::new(self.clone()))
    }

    pub fn recent_tracks_from_page(
        &self,
        starting_page: u32,
    ) -> Box<dyn AsyncPaginatedIterator<Track>> {
        Box::new(ApiRecentTracksIterator::with_starting_page(
            self.clone(),
            starting_page,
        ))
    }

    /// Iterate over recent tracks restricted to a unix-timestamp window.
    ///
    /// `from` and `to` are forwarded to the `user.getRecentTracks` endpoint's optional
    /// query parameters. Observed live (see the `api_recent_tracks_in_range` VCR test):
    /// `from` is **inclusive** and `to` is **exclusive** — a native half-open
    /// `[from, to)` window.
    pub fn recent_tracks_in_range(
        &self,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Box<dyn AsyncPaginatedIterator<Track>> {
        Box::new(ApiRecentTracksIterator::with_range(self.clone(), from, to))
    }
}

/// Build the `user.getRecentTracks` request URL, appending `from`/`to` only when present.
pub(crate) fn build_recent_tracks_url(
    username: &str,
    api_key: &str,
    page: u32,
    from: Option<u64>,
    to: Option<u64>,
) -> String {
    let mut url = format!(
        "https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user={}&api_key={}&format=json&page={}&limit=200",
        urlencoding::encode(username),
        urlencoding::encode(api_key),
        page
    );

    if let Some(from) = from {
        url.push_str(&format!("&from={from}"));
    }
    if let Some(to) = to {
        url.push_str(&format!("&to={to}"));
    }

    url
}

/// Shared implementation of a single `user.getRecentTracks` page fetch.
///
/// Builds the request URL (including the optional `from`/`to` unix-timestamp window),
/// broadcasts `RequestStarted`/`RequestCompleted` events, and parses the JSON response.
/// Used by both [`LastFmApiClientImpl`] and `LastFmEditClientImpl` so the request logic
/// exists in exactly one place.
pub(crate) async fn fetch_recent_tracks_page(
    client: &Arc<dyn HttpClient + Send + Sync>,
    broadcaster: &SharedEventBroadcaster,
    username: &str,
    api_key: &str,
    page: u32,
    from: Option<u64>,
    to: Option<u64>,
) -> Result<TrackPage> {
    let url = build_recent_tracks_url(username, api_key, page, from, to);

    let request_info = RequestInfo::from_url_and_method(&url, "GET");
    let request_start = std::time::Instant::now();

    broadcaster.broadcast_event(ClientEvent::RequestStarted {
        request: request_info.clone(),
    });

    let request = Request::new(Method::Get, url.parse::<Url>().unwrap());
    let mut response = client
        .send(request)
        .await
        .map_err(|e| LastFmError::Http(e.to_string()))?;

    broadcaster.broadcast_event(ClientEvent::RequestCompleted {
        request: request_info,
        status_code: response.status().into(),
        duration_ms: request_start.elapsed().as_millis() as u64,
    });

    let body = response
        .body_string()
        .await
        .map_err(|e| LastFmError::Http(e.to_string()))?;

    parse_api_recent_tracks_response(&body)
}

#[async_trait(?Send)]
impl LastFmApiClient for LastFmApiClientImpl {
    async fn api_get_recent_tracks_page_in_range(
        &self,
        page: u32,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Result<TrackPage> {
        fetch_recent_tracks_page(
            &self.client,
            &self.broadcaster,
            &self.username,
            &self.api_key,
            page,
            from,
            to,
        )
        .await
    }
}

#[derive(Deserialize)]
pub struct ApiRecentTracksResponse {
    pub recenttracks: ApiRecentTracks,
}

#[derive(Deserialize)]
pub struct ApiRecentTracks {
    /// The API serializes a single-track page as a bare object rather than a one-element
    /// array, and omits the field entirely for empty pages — accept all three shapes.
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    pub track: Vec<ApiTrack>,
    #[serde(rename = "@attr")]
    pub attr: ApiPaginationAttr,
}

fn deserialize_one_or_many<'de, D>(deserializer: D) -> std::result::Result<Vec<ApiTrack>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<ApiTrack>),
        One(Box<ApiTrack>),
    }
    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::Many(tracks)) => tracks,
        Some(OneOrMany::One(track)) => vec![*track],
    })
}

/// Error body shape returned by the API (e.g. `{"error":6,"message":"User not found"}`).
#[derive(Deserialize)]
struct ApiErrorResponse {
    error: i64,
    message: String,
}

#[derive(Deserialize)]
pub struct ApiTrack {
    pub name: String,
    pub artist: ApiTextField,
    pub album: ApiTextField,
    pub date: Option<ApiDate>,
    #[serde(rename = "@attr")]
    pub attr: Option<ApiTrackAttr>,
}

#[derive(Deserialize)]
pub struct ApiTextField {
    #[serde(rename = "#text")]
    pub text: String,
}

#[derive(Deserialize)]
pub struct ApiDate {
    pub uts: String,
}

#[derive(Deserialize)]
pub struct ApiTrackAttr {
    pub nowplaying: Option<String>,
}

#[derive(Deserialize)]
pub struct ApiPaginationAttr {
    pub page: String,
    #[serde(rename = "totalPages")]
    pub total_pages: String,
}

pub fn parse_api_recent_tracks_response(json: &str) -> Result<TrackPage> {
    let response: ApiRecentTracksResponse = serde_json::from_str(json).map_err(|e| {
        // Prefer surfacing the API's own error message when the body is an error payload.
        if let Ok(api_error) = serde_json::from_str::<ApiErrorResponse>(json) {
            crate::types::LastFmError::Http(format!(
                "last.fm API error {}: {}",
                api_error.error, api_error.message
            ))
        } else {
            crate::types::LastFmError::Parse(e.to_string())
        }
    })?;

    let current_page: u32 = response.recenttracks.attr.page.parse().unwrap_or(1);
    let total_pages: u32 = response.recenttracks.attr.total_pages.parse().unwrap_or(1);

    let tracks: Vec<Track> = response
        .recenttracks
        .track
        .into_iter()
        .filter(|t| {
            // Skip "now playing" tracks (they have no timestamp)
            if let Some(ref attr) = t.attr {
                if attr.nowplaying.as_deref() == Some("true") {
                    return false;
                }
            }
            true
        })
        .filter_map(|t| {
            let timestamp: u64 = t.date.as_ref()?.uts.parse().ok()?;
            Some(Track {
                name: t.name,
                artist: t.artist.text,
                playcount: 1,
                timestamp: Some(timestamp),
                album: Some(t.album.text),
                // The recent-tracks API response carries no album-artist field; report that
                // honestly instead of guessing. Scraped edit-form values are the authoritative
                // way to obtain it.
                album_artist: None,
            })
        })
        .collect();

    let has_next_page = current_page < total_pages;

    Ok(TrackPage {
        tracks,
        page_number: current_page,
        has_next_page,
        total_pages: Some(total_pages),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_api_recent_tracks() {
        let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "Test Track",
                        "artist": {"#text": "Test Artist"},
                        "album": {"#text": "Test Album"},
                        "date": {"uts": "1700000000"}
                    },
                    {
                        "name": "Now Playing",
                        "artist": {"#text": "Some Artist"},
                        "album": {"#text": "Some Album"},
                        "@attr": {"nowplaying": "true"}
                    }
                ],
                "@attr": {
                    "page": "1",
                    "totalPages": "5"
                }
            }
        }"##;

        let page = parse_api_recent_tracks_response(json).unwrap();
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(page.tracks[0].name, "Test Track");
        assert_eq!(page.tracks[0].artist, "Test Artist");
        assert_eq!(page.tracks[0].album.as_deref(), Some("Test Album"));
        // The API provides no album-artist information; it must not be fabricated.
        assert_eq!(page.tracks[0].album_artist, None);
        assert_eq!(page.tracks[0].timestamp, Some(1700000000));
        assert_eq!(page.tracks[0].playcount, 1);
        assert_eq!(page.page_number, 1);
        assert!(page.has_next_page);
        assert_eq!(page.total_pages, Some(5));
    }

    #[test]
    fn test_parse_single_track_as_object() {
        // The API returns a bare object (not a one-element array) for single-track pages.
        let json = r##"{
            "recenttracks": {
                "track": {
                    "name": "Solo",
                    "artist": {"#text": "Artist"},
                    "album": {"#text": "Album"},
                    "date": {"uts": "1700000000"}
                },
                "@attr": {"page": "1", "totalPages": "1"}
            }
        }"##;
        let page = parse_api_recent_tracks_response(json).unwrap();
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(page.tracks[0].name, "Solo");
    }

    #[test]
    fn test_parse_empty_page_without_track_field() {
        let json = r##"{
            "recenttracks": {
                "@attr": {"page": "1", "totalPages": "0"}
            }
        }"##;
        let page = parse_api_recent_tracks_response(json).unwrap();
        assert!(page.tracks.is_empty());
    }

    #[test]
    fn test_api_error_body_is_surfaced() {
        let json = r##"{"error":6,"message":"User not found"}"##;
        let err = parse_api_recent_tracks_response(json).unwrap_err();
        assert!(err.to_string().contains("User not found"), "{err}");
    }

    #[test]
    fn test_parse_api_last_page() {
        let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "Track",
                        "artist": {"#text": "Artist"},
                        "album": {"#text": "Album"},
                        "date": {"uts": "1700000000"}
                    }
                ],
                "@attr": {
                    "page": "3",
                    "totalPages": "3"
                }
            }
        }"##;

        let page = parse_api_recent_tracks_response(json).unwrap();
        assert!(!page.has_next_page);
        assert_eq!(page.page_number, 3);
    }

    #[test]
    fn test_build_recent_tracks_url_without_range() {
        let url = build_recent_tracks_url("someuser", "apikey123", 2, None, None);
        assert_eq!(
            url,
            "https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user=someuser&api_key=apikey123&format=json&page=2&limit=200"
        );
        assert!(!url.contains("&from="));
        assert!(!url.contains("&to="));
    }

    #[test]
    fn test_build_recent_tracks_url_with_from_and_to() {
        let url = build_recent_tracks_url(
            "someuser",
            "apikey123",
            1,
            Some(1700000000),
            Some(1700086400),
        );
        assert_eq!(
            url,
            "https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user=someuser&api_key=apikey123&format=json&page=1&limit=200&from=1700000000&to=1700086400"
        );
    }

    #[test]
    fn test_build_recent_tracks_url_with_only_from() {
        let url = build_recent_tracks_url("someuser", "apikey123", 1, Some(1700000000), None);
        assert!(url.ends_with("&limit=200&from=1700000000"));
        assert!(!url.contains("&to="));
    }

    #[test]
    fn test_build_recent_tracks_url_with_only_to() {
        let url = build_recent_tracks_url("someuser", "apikey123", 1, None, Some(1700086400));
        assert!(url.ends_with("&limit=200&to=1700086400"));
        assert!(!url.contains("&from="));
    }

    #[test]
    fn test_build_recent_tracks_url_encodes_username() {
        let url = build_recent_tracks_url("some user&x", "key", 1, None, None);
        assert!(url.contains("user=some%20user%26x"));
    }
}
