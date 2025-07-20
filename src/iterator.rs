use crate::{LastFmClient, Result, Track, TrackPage};

pub struct ArtistTracksIterator<'a> {
    client: &'a mut LastFmClient,
    artist: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Track>,
    total_pages: Option<u32>,
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

    pub async fn collect_all(&mut self) -> Result<Vec<Track>> {
        let mut all_tracks = Vec::new();

        while let Some(page) = self.next_page().await? {
            all_tracks.extend(page.tracks);
        }

        Ok(all_tracks)
    }

    pub async fn take(&mut self, n: usize) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();

        while tracks.len() < n {
            if self.buffer.is_empty() {
                match self.next_page().await? {
                    Some(page) => self.buffer = page.tracks,
                    None => break,
                }
            }

            let remaining = n - tracks.len();
            if self.buffer.len() <= remaining {
                tracks.extend(self.buffer.drain(..));
            } else {
                tracks.extend(self.buffer.drain(..remaining));
            }
        }

        Ok(tracks)
    }

    pub fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }

    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}