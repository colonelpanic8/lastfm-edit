//! Album covers and artist photos from last.fm, resolved to image URLs the webview loads
//! directly and cached on disk so each subject is looked up once.
//!
//! Covers come from the JSON API's `album.getInfo` when an API key is configured. Artist
//! photos only exist on the website (the API has served a placeholder since 2019), so
//! those, and covers without an API key, are read from the page's `og:image` using the
//! saved session. Page fetches are serialized and paused on throttling so they never
//! compete hard with the edit executor for the session's rate limit.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{OnceCell, Semaphore};

const API_ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
/// last.fm's "no image" star and blank-record images.
const PLACEHOLDERS: [&str; 2] = [
    "2a96cbd8b46e442fc41c2b86b821562f",
    "c6f59c1e5e7240a4c0d427abd71f3dbb",
];
const PAGE_GAP: Duration = Duration::from_millis(400);
/// Two permits each held this long keep lookups within the API's 5 requests/second.
const API_SLOT: Duration = Duration::from_millis(400);
const THROTTLE_PAUSE: Duration = Duration::from_secs(300);
const MISS_TTL_SECS: u64 = 30 * 86_400;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ArtSubject {
    Artist(String),
    Album { artist: String, album: String },
}

impl ArtSubject {
    /// The album cover when the album is known, else the artist photo.
    pub fn for_track(artist: &str, album: Option<&str>) -> Self {
        match album.filter(|album| !album.trim().is_empty()) {
            Some(album) => Self::Album {
                artist: artist.to_string(),
                album: album.to_string(),
            },
            None => Self::Artist(artist.to_string()),
        }
    }

    /// The text a placeholder tile derives its initial and colour from.
    pub fn label(&self) -> &str {
        match self {
            Self::Artist(artist) => artist,
            Self::Album { album, .. } => album,
        }
    }

    fn cache_key(&self) -> String {
        match self {
            Self::Artist(artist) => format!("artist\u{1f}{}", artist.to_lowercase()),
            Self::Album { artist, album } => format!(
                "album\u{1f}{}\u{1f}{}",
                artist.to_lowercase(),
                album.to_lowercase()
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedArt {
    url: Option<String>,
    checked_at: u64,
}

/// A failure worth retrying later; definitive "no image" answers are `Ok(None)`.
#[derive(Debug)]
struct Transient(String);

impl<E: std::fmt::Display> From<E> for Transient {
    fn from(error: E) -> Self {
        Self(error.to_string())
    }
}

pub struct ArtworkResolver {
    http: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    cookie_header: String,
    cache_path: PathBuf,
    resolved: Mutex<HashMap<String, Arc<OnceCell<Option<String>>>>>,
    persisted: Mutex<HashMap<String, CachedArt>>,
    api_permits: Semaphore,
    /// Held across each page fetch; the value is when throttling ends.
    page_gate: tokio::sync::Mutex<Option<Instant>>,
}

impl ArtworkResolver {
    pub fn new(
        session: &lastfm_edit::LastFmEditSession,
        api_key: Option<String>,
        cache_path: PathBuf,
    ) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| format!("could not initialize artwork client: {error}"))?;
        let persisted: HashMap<String, CachedArt> = std::fs::read_to_string(&cache_path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default();
        let now = unix_now();
        let resolved = persisted
            .iter()
            .filter(|(_, entry)| {
                entry.url.is_some() || now.saturating_sub(entry.checked_at) < MISS_TTL_SECS
            })
            .map(|(key, entry)| {
                (
                    key.clone(),
                    Arc::new(OnceCell::new_with(Some(entry.url.clone()))),
                )
            })
            .collect();
        Ok(Self {
            http,
            api_key,
            base_url: session.base_url.trim_end_matches('/').to_string(),
            cookie_header: session.cookies.join("; "),
            cache_path,
            resolved: Mutex::new(resolved),
            persisted: Mutex::new(persisted),
            api_permits: Semaphore::new(2),
            page_gate: tokio::sync::Mutex::new(None),
        })
    }

    /// The image URL for `subject`, or `None` when last.fm has none or the lookup failed
    /// (failures are retried the next time the subject is shown).
    pub async fn resolve(self: Arc<Self>, subject: ArtSubject) -> Option<String> {
        let key = subject.cache_key();
        let cell = self
            .resolved
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone();
        let lookup = cell.get_or_try_init(|| async {
            let url = self.lookup(&subject).await?;
            self.remember(key, url.clone());
            Ok::<_, Transient>(url)
        });
        match lookup.await {
            Ok(url) => url.clone(),
            Err(Transient(error)) => {
                tracing::debug!(?subject, %error, "artwork lookup failed");
                None
            }
        }
    }

    async fn lookup(&self, subject: &ArtSubject) -> Result<Option<String>, Transient> {
        match (subject, &self.api_key) {
            (ArtSubject::Album { artist, album }, Some(api_key)) => {
                self.album_from_api(api_key, artist, album).await
            }
            (ArtSubject::Album { artist, album }, None) => {
                self.og_image(&format!("/music/{}/{}", encode(artist), encode(album)))
                    .await
            }
            (ArtSubject::Artist(artist), _) => {
                self.og_image(&format!("/music/{}", encode(artist))).await
            }
        }
    }

    async fn album_from_api(
        &self,
        api_key: &str,
        artist: &str,
        album: &str,
    ) -> Result<Option<String>, Transient> {
        let _permit = self.api_permits.acquire().await?;
        let request = async {
            self.http
                .get(API_ROOT)
                .query(&[
                    ("method", "album.getinfo"),
                    ("artist", artist),
                    ("album", album),
                    ("autocorrect", "1"),
                    ("api_key", api_key),
                    ("format", "json"),
                ])
                .send()
                .await?
                .text()
                .await
        };
        let (body, ()) = tokio::join!(request, tokio::time::sleep(API_SLOT));
        parse_album_info(&body?)
    }

    async fn og_image(&self, path: &str) -> Result<Option<String>, Transient> {
        let mut gate = self.page_gate.lock().await;
        if let Some(until) = *gate {
            if Instant::now() < until {
                return Err(Transient("page lookups paused after throttling".into()));
            }
        }
        let response = self
            .http
            .get(format!("{}{path}", self.base_url))
            .header("Cookie", &self.cookie_header)
            .header("Accept", "text/html,application/xhtml+xml")
            .send()
            .await;
        tokio::time::sleep(PAGE_GAP).await;
        let response = response?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            *gate = Some(Instant::now() + THROTTLE_PAUSE);
            return Err(Transient(format!("last.fm answered {status}")));
        }
        let body = response.error_for_status()?.text().await?;
        parse_og_image(&body).ok_or_else(|| Transient("page has no og:image".into()))
    }

    fn remember(&self, key: String, url: Option<String>) {
        let mut persisted = self.persisted.lock().unwrap();
        persisted.insert(
            key,
            CachedArt {
                url,
                checked_at: unix_now(),
            },
        );
        let partial = self.cache_path.with_extension("json.tmp");
        let written = serde_json::to_string(&*persisted)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                std::fs::write(&partial, json)
                    .and_then(|()| std::fs::rename(&partial, &self.cache_path))
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = written {
            tracing::warn!(%error, "could not persist artwork cache");
        }
    }
}

fn encode(segment: &str) -> String {
    urlencoding::encode(segment).into_owned()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Rewrites a last.fm image URL to its 300px rendition, or drops a placeholder.
fn usable_image(url: &str) -> Option<String> {
    if url.is_empty() || PLACEHOLDERS.iter().any(|hash| url.contains(hash)) {
        return None;
    }
    let Some((prefix, rest)) = url.split_once("/i/u/") else {
        return Some(url.to_string());
    };
    let file = rest.rsplit('/').next().unwrap_or(rest);
    Some(format!("{prefix}/i/u/300x300/{file}"))
}

fn parse_album_info(body: &str) -> Result<Option<String>, Transient> {
    #[derive(Deserialize)]
    struct Envelope {
        album: Option<Album>,
        error: Option<u32>,
        message: Option<String>,
    }
    #[derive(Deserialize)]
    struct Album {
        #[serde(default)]
        image: Vec<Image>,
    }
    #[derive(Deserialize)]
    struct Image {
        #[serde(rename = "#text")]
        url: String,
    }

    const NOT_FOUND: u32 = 6;
    let envelope: Envelope = serde_json::from_str(body)?;
    match (envelope.album, envelope.error) {
        (Some(album), _) => Ok(album
            .image
            .iter()
            .rev()
            .find_map(|image| usable_image(&image.url))),
        (None, Some(NOT_FOUND)) => Ok(None),
        (None, error) => Err(Transient(format!(
            "album.getinfo error {error:?}: {}",
            envelope.message.unwrap_or_default()
        ))),
    }
}

/// `Some(None)` for a page whose `og:image` is a placeholder; `None` when the page has no
/// `og:image` at all (a bot challenge or logged-out shell, not an answer).
fn parse_og_image(html: &str) -> Option<Option<String>> {
    let tag_start = html.find("property=\"og:image\"")?;
    let tag = &html[tag_start..];
    let tag = &tag[..tag.find('>')?];
    let content = tag.split_once("content=\"")?.1;
    let url = &content[..content.find('"')?];
    Some(usable_image(url))
}

#[cfg(test)]
#[path = "artwork_tests.rs"]
mod tests;
