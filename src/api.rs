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
    async fn api_get_recent_tracks_page(&self, page: u32) -> Result<TrackPage>;
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
}

#[async_trait(?Send)]
impl LastFmApiClient for LastFmApiClientImpl {
    async fn api_get_recent_tracks_page(&self, page: u32) -> Result<TrackPage> {
        let url = format!(
            "https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user={}&api_key={}&format=json&page={}&limit=200",
            urlencoding::encode(&self.username),
            urlencoding::encode(&self.api_key),
            page
        );

        let request_info = RequestInfo::from_url_and_method(&url, "GET");
        let request_start = std::time::Instant::now();

        self.broadcaster
            .broadcast_event(ClientEvent::RequestStarted {
                request: request_info.clone(),
            });

        let request = Request::new(Method::Get, url.parse::<Url>().unwrap());
        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.broadcaster
            .broadcast_event(ClientEvent::RequestCompleted {
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
}

#[derive(Deserialize)]
pub struct ApiRecentTracksResponse {
    pub recenttracks: ApiRecentTracks,
}

#[derive(Deserialize)]
pub struct ApiRecentTracks {
    pub track: Vec<ApiTrack>,
    #[serde(rename = "@attr")]
    pub attr: ApiPaginationAttr,
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
    let response: ApiRecentTracksResponse =
        serde_json::from_str(json).map_err(|e| crate::types::LastFmError::Parse(e.to_string()))?;

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
            let artist = t.artist.text.clone();
            Some(Track {
                name: t.name,
                artist: artist.clone(),
                playcount: 1,
                timestamp: Some(timestamp),
                album: Some(t.album.text),
                album_artist: Some(artist),
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
        assert_eq!(page.tracks[0].timestamp, Some(1700000000));
        assert_eq!(page.tracks[0].playcount, 1);
        assert_eq!(page.page_number, 1);
        assert!(page.has_next_page);
        assert_eq!(page.total_pages, Some(5));
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
}
