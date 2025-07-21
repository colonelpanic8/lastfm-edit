use crate::{Album, AlbumPage, LastFmClient, Result, Track, TrackPage};

/// Async iterator trait for paginated Last.fm data
#[allow(async_fn_in_trait)]
pub trait AsyncPaginatedIterator {
    /// The item type yielded by this iterator
    type Item;

    /// Fetch the next item from the iterator
    async fn next(&mut self) -> Result<Option<Self::Item>>;

    /// Collect all remaining items into a Vec
    async fn collect_all(&mut self) -> Result<Vec<Self::Item>> {
        let mut items = Vec::new();
        while let Some(item) = self.next().await? {
            items.push(item);
        }
        Ok(items)
    }

    /// Take up to n items from the iterator
    async fn take(&mut self, n: usize) -> Result<Vec<Self::Item>> {
        let mut items = Vec::new();
        for _ in 0..n {
            match self.next().await? {
                Some(item) => items.push(item),
                None => break,
            }
        }
        Ok(items)
    }

    /// Get the current page number (0-indexed)
    fn current_page(&self) -> u32;
}

pub struct ArtistTracksIterator<'a> {
    client: &'a mut LastFmClient,
    artist: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Track>,
    total_pages: Option<u32>,
}

impl<'a> AsyncPaginatedIterator for ArtistTracksIterator<'a> {
    type Item = Track;

    async fn next(&mut self) -> Result<Option<Self::Item>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.tracks;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        Ok(self.buffer.pop())
    }

    fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }
}

impl<'a> ArtistTracksIterator<'a> {
    pub fn new(client: &'a mut LastFmClient, artist: String) -> Self {
        Self {
            client,
            artist,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    pub async fn next_page(&mut self) -> Result<Option<TrackPage>> {
        if !self.has_more {
            return Ok(None);
        }

        // Add a small delay for paginated requests to be polite to the server
        if self.current_page > 1 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        let page = self
            .client
            .get_artist_tracks_page(&self.artist, self.current_page)
            .await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

pub struct ArtistAlbumsIterator<'a> {
    client: &'a mut LastFmClient,
    artist: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Album>,
    total_pages: Option<u32>,
}

impl<'a> AsyncPaginatedIterator for ArtistAlbumsIterator<'a> {
    type Item = Album;

    async fn next(&mut self) -> Result<Option<Self::Item>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.albums;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        Ok(self.buffer.pop())
    }

    fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }
}

impl<'a> ArtistAlbumsIterator<'a> {
    pub fn new(client: &'a mut LastFmClient, artist: String) -> Self {
        Self {
            client,
            artist,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    pub async fn next_page(&mut self) -> Result<Option<AlbumPage>> {
        if !self.has_more {
            return Ok(None);
        }

        // Add a small delay for paginated requests to be polite to the server
        if self.current_page > 1 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        let page = self
            .client
            .get_artist_albums_page(&self.artist, self.current_page)
            .await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

pub struct RecentTracksIterator<'a> {
    client: &'a mut LastFmClient,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Track>,
    stop_at_timestamp: Option<u64>,
}

impl<'a> AsyncPaginatedIterator for RecentTracksIterator<'a> {
    type Item = Track;

    async fn next(&mut self) -> Result<Option<Self::Item>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if !self.has_more {
                return Ok(None);
            }

            // Add a small delay for paginated requests to be polite to the server
            if self.current_page > 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }

            let tracks = self.client.get_recent_scrobbles(self.current_page).await?;

            if tracks.is_empty() {
                self.has_more = false;
                return Ok(None);
            }

            // Check if we should stop based on timestamp
            if let Some(stop_timestamp) = self.stop_at_timestamp {
                let mut filtered_tracks = Vec::new();
                for track in tracks {
                    if let Some(track_timestamp) = track.timestamp {
                        if track_timestamp <= stop_timestamp {
                            self.has_more = false;
                            break;
                        }
                    }
                    filtered_tracks.push(track);
                }
                self.buffer = filtered_tracks;
            } else {
                self.buffer = tracks;
            }

            self.buffer.reverse(); // Reverse so we can pop from end efficiently
            self.current_page += 1;
        }

        Ok(self.buffer.pop())
    }

    fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }
}

impl<'a> RecentTracksIterator<'a> {
    pub fn new(client: &'a mut LastFmClient) -> Self {
        Self {
            client,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            stop_at_timestamp: None,
        }
    }

    pub fn with_stop_timestamp(mut self, timestamp: u64) -> Self {
        self.stop_at_timestamp = Some(timestamp);
        self
    }
}
