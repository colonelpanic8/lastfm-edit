use crate::{Album, AlbumPage, LastFmClient, Result, Track, TrackPage};

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

    /// Async method to fetch next track from the iterator
    pub async fn next(&mut self) -> Result<Option<Track>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.tracks;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        Ok(self.buffer.pop())
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

        while let Some(track) = self.next().await? {
            all_tracks.push(track);
        }

        Ok(all_tracks)
    }

    pub async fn take(&mut self, n: usize) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();

        for _ in 0..n {
            match self.next().await? {
                Some(track) => tracks.push(track),
                None => break,
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

pub struct ArtistAlbumsIterator<'a> {
    client: &'a mut LastFmClient,
    artist: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Album>,
    total_pages: Option<u32>,
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

    /// Async method to fetch next album from the iterator
    pub async fn next(&mut self) -> Result<Option<Album>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.albums;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        Ok(self.buffer.pop())
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

    pub async fn collect_all(&mut self) -> Result<Vec<Album>> {
        let mut all_albums = Vec::new();

        while let Some(album) = self.next().await? {
            all_albums.push(album);
        }

        Ok(all_albums)
    }

    pub async fn take(&mut self, n: usize) -> Result<Vec<Album>> {
        let mut albums = Vec::new();

        for _ in 0..n {
            match self.next().await? {
                Some(album) => albums.push(album),
                None => break,
            }
        }

        Ok(albums)
    }

    pub fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }

    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}
