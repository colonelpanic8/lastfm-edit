use crate::r#trait::LastFmEditClient;
use crate::{Album, AlbumPage, Result, Track, TrackPage};

use async_trait::async_trait;

/// Async iterator trait for paginated Last.fm data.
///
/// This trait provides a common interface for iterating over paginated data from Last.fm,
/// such as tracks, albums, and recent scrobbles. All iterators implement efficient streaming
/// with automatic pagination and built-in rate limiting.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait(?Send)]
pub trait AsyncPaginatedIterator<T> {
    /// Fetch the next item from the iterator.
    ///
    /// This method automatically handles pagination, fetching new pages as needed.
    /// Returns `None` when there are no more items available.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(item))` - Next item in the sequence
    /// - `Ok(None)` - No more items available
    /// - `Err(...)` - Network or parsing error occurred
    async fn next(&mut self) -> Result<Option<T>>;

    /// Collect all remaining items into a Vec.
    ///
    /// **Warning**: This method will fetch ALL remaining pages, which could be
    /// many thousands of items for large libraries. Use [`take`](Self::take) for
    /// safer bounded collection.
    async fn collect_all(&mut self) -> Result<Vec<T>> {
        let mut items = Vec::new();
        while let Some(item) = self.next().await? {
            items.push(item);
        }
        Ok(items)
    }

    /// Take up to n items from the iterator.
    ///
    /// This is the recommended way to collect a bounded number of items
    /// from potentially large datasets.
    ///
    /// # Arguments
    ///
    /// * `n` - Maximum number of items to collect
    async fn take(&mut self, n: usize) -> Result<Vec<T>> {
        let mut items = Vec::new();
        for _ in 0..n {
            match self.next().await? {
                Some(item) => items.push(item),
                None => break,
            }
        }
        Ok(items)
    }

    /// Get the current page number (0-indexed).
    ///
    /// Returns the page number of the most recently fetched page.
    fn current_page(&self) -> u32;

    /// Get the total number of pages, if known.
    ///
    /// Returns `Some(n)` if the total page count is known, `None` otherwise.
    /// This information may not be available until at least one page has been fetched.
    fn total_pages(&self) -> Option<u32> {
        None // Default implementation returns None
    }
}

/// Iterator for browsing an artist's tracks from a user's library.
///
/// This iterator provides access to all tracks by a specific artist
/// in the authenticated user's Last.fm library. Unlike the basic track listing,
/// this iterator fetches tracks by iterating through the artist's albums first,
/// which provides complete album information for each track.
///
/// The iterator loads albums and their tracks as needed and handles rate limiting
/// automatically to be respectful to Last.fm's servers.
pub struct ArtistTracksIterator<C: LastFmEditClient> {
    client: C,
    artist: String,
    album_iterator: Option<ArtistAlbumsIterator<C>>,
    current_album_tracks: Option<AlbumTracksIterator<C>>,
    track_buffer: Vec<Track>,
    finished: bool,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient + Clone> AsyncPaginatedIterator<Track> for ArtistTracksIterator<C> {
    async fn next(&mut self) -> Result<Option<Track>> {
        // If we're finished, return None
        if self.finished {
            return Ok(None);
        }

        // If track buffer is empty, try to get more tracks
        while self.track_buffer.is_empty() {
            // If we don't have a current album tracks iterator, get the next album
            if self.current_album_tracks.is_none() {
                // Initialize album iterator if needed
                if self.album_iterator.is_none() {
                    self.album_iterator = Some(ArtistAlbumsIterator::new(
                        self.client.clone(),
                        self.artist.clone(),
                    ));
                }

                // Get next album
                if let Some(ref mut album_iter) = self.album_iterator {
                    if let Some(album) = album_iter.next().await? {
                        log::debug!(
                            "Processing album '{}' for artist '{}'",
                            album.name,
                            self.artist
                        );
                        // Create album tracks iterator for this album
                        self.current_album_tracks = Some(AlbumTracksIterator::new(
                            self.client.clone(),
                            album.name.clone(),
                            self.artist.clone(),
                        ));
                    } else {
                        // No more albums, we're done
                        log::debug!("No more albums for artist '{}'", self.artist);
                        self.finished = true;
                        return Ok(None);
                    }
                }
            }

            // Get tracks from current album
            if let Some(ref mut album_tracks) = self.current_album_tracks {
                if let Some(track) = album_tracks.next().await? {
                    self.track_buffer.push(track);
                } else {
                    // This album is exhausted, move to next album
                    log::debug!(
                        "Finished processing current album for artist '{}'",
                        self.artist
                    );
                    self.current_album_tracks = None;
                    // Continue the loop to try getting the next album
                }
            }
        }

        // Return the next track from our buffer
        Ok(self.track_buffer.pop())
    }

    fn current_page(&self) -> u32 {
        // Since we're iterating through albums, return the album iterator's current page
        if let Some(ref album_iter) = self.album_iterator {
            album_iter.current_page()
        } else {
            0
        }
    }

    fn total_pages(&self) -> Option<u32> {
        // Since we're iterating through albums, return the album iterator's total pages
        if let Some(ref album_iter) = self.album_iterator {
            album_iter.total_pages()
        } else {
            None
        }
    }
}

impl<C: LastFmEditClient + Clone> ArtistTracksIterator<C> {
    /// Create a new artist tracks iterator.
    ///
    /// This is typically called via [`LastFmEditClient::artist_tracks`](crate::LastFmEditClient::artist_tracks).
    pub fn new(client: C, artist: String) -> Self {
        Self {
            client,
            artist,
            album_iterator: None,
            current_album_tracks: None,
            track_buffer: Vec::new(),
            finished: false,
        }
    }
}

/// Iterator for browsing an artist's tracks directly using the paginated artist tracks endpoint.
///
/// This iterator provides access to all tracks by a specific artist
/// in the authenticated user's Last.fm library by directly using the
/// `/user/{username}/library/music/{artist}/+tracks` endpoint with pagination.
/// This is more efficient than the album-based approach as it doesn't need to
/// iterate through albums first.
pub struct ArtistTracksDirectIterator<C: LastFmEditClient> {
    client: C,
    artist: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Track>,
    total_pages: Option<u32>,
    tracks_yielded: u32,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<Track> for ArtistTracksDirectIterator<C> {
    async fn next(&mut self) -> Result<Option<Track>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.tracks;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        if let Some(track) = self.buffer.pop() {
            self.tracks_yielded += 1;
            Ok(Some(track))
        } else {
            Ok(None)
        }
    }

    fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }

    fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

impl<C: LastFmEditClient> ArtistTracksDirectIterator<C> {
    /// Create a new direct artist tracks iterator.
    ///
    /// This is typically called via [`LastFmEditClient::artist_tracks_direct`](crate::LastFmEditClient::artist_tracks_direct).
    pub fn new(client: C, artist: String) -> Self {
        Self {
            client,
            artist,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
            tracks_yielded: 0,
        }
    }

    /// Fetch the next page of tracks.
    ///
    /// This method handles pagination automatically and includes rate limiting.
    pub async fn next_page(&mut self) -> Result<Option<TrackPage>> {
        if !self.has_more {
            return Ok(None);
        }

        log::debug!(
            "Fetching page {} of {} tracks (yielded {} tracks so far)",
            self.current_page,
            self.artist,
            self.tracks_yielded
        );

        let page = self
            .client
            .get_artist_tracks_page(&self.artist, self.current_page)
            .await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    /// Get the total number of pages, if known.
    ///
    /// Returns `None` until at least one page has been fetched.
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

/// Iterator for browsing an artist's albums from a user's library.
///
/// This iterator provides paginated access to all albums by a specific artist
/// in the authenticated user's Last.fm library, ordered by play count.
pub struct ArtistAlbumsIterator<C: LastFmEditClient> {
    client: C,
    artist: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Album>,
    total_pages: Option<u32>,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<Album> for ArtistAlbumsIterator<C> {
    async fn next(&mut self) -> Result<Option<Album>> {
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

    fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

impl<C: LastFmEditClient> ArtistAlbumsIterator<C> {
    /// Create a new artist albums iterator.
    ///
    /// This is typically called via [`LastFmEditClient::artist_albums`](crate::LastFmEditClient::artist_albums).
    pub fn new(client: C, artist: String) -> Self {
        Self {
            client,
            artist,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Fetch the next page of albums.
    ///
    /// This method handles pagination automatically and includes rate limiting.
    pub async fn next_page(&mut self) -> Result<Option<AlbumPage>> {
        if !self.has_more {
            return Ok(None);
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

    /// Get the total number of pages, if known.
    ///
    /// Returns `None` until at least one page has been fetched.
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

/// Iterator for browsing a user's recent tracks/scrobbles.
///
/// This iterator provides access to the user's recent listening history with timestamps,
/// which is essential for finding tracks that can be edited. It supports optional
/// timestamp-based filtering to avoid reprocessing old data.
pub struct RecentTracksIterator<C: LastFmEditClient> {
    client: C,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Track>,
    stop_at_timestamp: Option<u64>,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<Track> for RecentTracksIterator<C> {
    async fn next(&mut self) -> Result<Option<Track>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if !self.has_more {
                return Ok(None);
            }

            let page = self
                .client
                .get_recent_tracks_page(self.current_page)
                .await?;

            if page.tracks.is_empty() {
                self.has_more = false;
                return Ok(None);
            }

            self.has_more = page.has_next_page;

            // Check if we should stop based on timestamp
            if let Some(stop_timestamp) = self.stop_at_timestamp {
                let mut filtered_tracks = Vec::new();
                for track in page.tracks {
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
                self.buffer = page.tracks;
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

impl<C: LastFmEditClient> RecentTracksIterator<C> {
    /// Create a new recent tracks iterator starting from page 1.
    ///
    /// This is typically called via [`LastFmEditClient::recent_tracks`](crate::LastFmEditClient::recent_tracks).
    pub fn new(client: C) -> Self {
        Self::with_starting_page(client, 1)
    }

    /// Create a new recent tracks iterator starting from a specific page.
    ///
    /// This allows resuming pagination from an arbitrary page, useful for
    /// continuing from where a previous iteration left off.
    ///
    /// # Arguments
    ///
    /// * `client` - The LastFmEditClient to use for API calls
    /// * `starting_page` - The page number to start from (1-indexed)
    pub fn with_starting_page(client: C, starting_page: u32) -> Self {
        let page = std::cmp::max(1, starting_page);
        Self {
            client,
            current_page: page,
            has_more: true,
            buffer: Vec::new(),
            stop_at_timestamp: None,
        }
    }

    /// Set a timestamp to stop iteration at.
    ///
    /// When this is set, the iterator will stop returning tracks once it encounters
    /// a track with a timestamp less than or equal to the specified value. This is
    /// useful for incremental processing to avoid reprocessing old data.
    ///
    /// # Arguments
    ///
    /// * `timestamp` - Unix timestamp to stop at
    pub fn with_stop_timestamp(mut self, timestamp: u64) -> Self {
        self.stop_at_timestamp = Some(timestamp);
        self
    }
}

/// Iterator for browsing tracks in a specific album from a user's library.
///
/// This iterator provides access to all tracks in a specific album by an artist
/// in the authenticated user's Last.fm library. Unlike paginated iterators,
/// this loads tracks once and iterates through them.
pub struct AlbumTracksIterator<C: LastFmEditClient> {
    client: C,
    album_name: String,
    artist_name: String,
    tracks: Option<Vec<Track>>,
    index: usize,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<Track> for AlbumTracksIterator<C> {
    async fn next(&mut self) -> Result<Option<Track>> {
        // Load tracks if not already loaded
        if self.tracks.is_none() {
            // Use get_album_tracks_page instead of get_album_tracks to avoid infinite recursion
            let tracks_page = self
                .client
                .get_album_tracks_page(&self.album_name, &self.artist_name, 1)
                .await?;
            log::debug!(
                "Album '{}' by '{}' has {} tracks: {:?}",
                self.album_name,
                self.artist_name,
                tracks_page.tracks.len(),
                tracks_page
                    .tracks
                    .iter()
                    .map(|t| &t.name)
                    .collect::<Vec<_>>()
            );

            if tracks_page.tracks.is_empty() {
                log::warn!(
                    "🚨 ZERO TRACKS FOUND for album '{}' by '{}' - investigating...",
                    self.album_name,
                    self.artist_name
                );
                log::debug!("Full TrackPage for empty album: has_next_page={}, page_number={}, total_pages={:?}",
                           tracks_page.has_next_page, tracks_page.page_number, tracks_page.total_pages);
            }
            self.tracks = Some(tracks_page.tracks);
        }

        // Return next track
        if let Some(tracks) = &self.tracks {
            if self.index < tracks.len() {
                let track = tracks[self.index].clone();
                self.index += 1;
                Ok(Some(track))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    fn current_page(&self) -> u32 {
        // Album tracks don't have pages, so return 0
        0
    }
}

impl<C: LastFmEditClient> AlbumTracksIterator<C> {
    /// Create a new album tracks iterator.
    ///
    /// This is typically called via [`LastFmEditClient::album_tracks`](crate::LastFmEditClient::album_tracks).
    pub fn new(client: C, album_name: String, artist_name: String) -> Self {
        Self {
            client,
            album_name,
            artist_name,
            tracks: None,
            index: 0,
        }
    }
}

/// Iterator for searching tracks in the user's library.
///
/// This iterator provides paginated access to tracks that match a search query
/// in the authenticated user's Last.fm library, using Last.fm's built-in search functionality.
pub struct SearchTracksIterator<C: LastFmEditClient> {
    client: C,
    query: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Track>,
    total_pages: Option<u32>,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<Track> for SearchTracksIterator<C> {
    async fn next(&mut self) -> Result<Option<Track>> {
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

    fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

impl<C: LastFmEditClient> SearchTracksIterator<C> {
    /// Create a new search tracks iterator.
    ///
    /// This is typically called via [`LastFmEditClient::search_tracks`](crate::LastFmEditClient::search_tracks).
    pub fn new(client: C, query: String) -> Self {
        Self {
            client,
            query,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Create a new search tracks iterator starting from a specific page.
    ///
    /// This is useful for implementing offset functionality efficiently by starting
    /// at the appropriate page rather than iterating through all previous pages.
    pub fn with_starting_page(client: C, query: String, starting_page: u32) -> Self {
        let page = std::cmp::max(1, starting_page);
        Self {
            client,
            query,
            current_page: page,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Fetch the next page of search results.
    ///
    /// This method handles pagination automatically and includes rate limiting
    /// to be respectful to Last.fm's servers.
    pub async fn next_page(&mut self) -> Result<Option<TrackPage>> {
        if !self.has_more {
            return Ok(None);
        }

        let page = self
            .client
            .search_tracks_page(&self.query, self.current_page)
            .await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    /// Get the total number of pages, if known.
    ///
    /// Returns `None` until at least one page has been fetched.
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

/// Iterator for searching albums in the user's library.
///
/// This iterator provides paginated access to albums that match a search query
/// in the authenticated user's Last.fm library, using Last.fm's built-in search functionality.
///
/// # Examples
pub struct SearchAlbumsIterator<C: LastFmEditClient> {
    client: C,
    query: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<Album>,
    total_pages: Option<u32>,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<Album> for SearchAlbumsIterator<C> {
    async fn next(&mut self) -> Result<Option<Album>> {
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

    fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

impl<C: LastFmEditClient> SearchAlbumsIterator<C> {
    /// Create a new search albums iterator.
    ///
    /// This is typically called via [`LastFmEditClient::search_albums`](crate::LastFmEditClient::search_albums).
    pub fn new(client: C, query: String) -> Self {
        Self {
            client,
            query,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Create a new search albums iterator starting from a specific page.
    ///
    /// This is useful for implementing offset functionality efficiently by starting
    /// at the appropriate page rather than iterating through all previous pages.
    pub fn with_starting_page(client: C, query: String, starting_page: u32) -> Self {
        let page = std::cmp::max(1, starting_page);
        Self {
            client,
            query,
            current_page: page,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Fetch the next page of search results.
    ///
    /// This method handles pagination automatically and includes rate limiting
    /// to be respectful to Last.fm's servers.
    pub async fn next_page(&mut self) -> Result<Option<AlbumPage>> {
        if !self.has_more {
            return Ok(None);
        }

        let page = self
            .client
            .search_albums_page(&self.query, self.current_page)
            .await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    /// Get the total number of pages, if known.
    ///
    /// Returns `None` until at least one page has been fetched.
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

/// Iterator for searching artists in the user's library.
///
/// This iterator provides paginated access to artists that match a search query
/// in the authenticated user's Last.fm library, using Last.fm's built-in search functionality.
pub struct SearchArtistsIterator<C: LastFmEditClient> {
    client: C,
    query: String,
    current_page: u32,
    has_more: bool,
    buffer: Vec<crate::Artist>,
    total_pages: Option<u32>,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<crate::Artist> for SearchArtistsIterator<C> {
    async fn next(&mut self) -> Result<Option<crate::Artist>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.artists;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        Ok(self.buffer.pop())
    }

    fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }

    fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

impl<C: LastFmEditClient> SearchArtistsIterator<C> {
    /// Create a new search artists iterator.
    ///
    /// This is typically called via [`LastFmEditClient::search_artists`](crate::LastFmEditClient::search_artists).
    pub fn new(client: C, query: String) -> Self {
        Self {
            client,
            query,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Create a new search artists iterator starting from a specific page.
    ///
    /// This is useful for implementing offset functionality efficiently by starting
    /// at the appropriate page rather than iterating through all previous pages.
    pub fn with_starting_page(client: C, query: String, starting_page: u32) -> Self {
        let page = std::cmp::max(1, starting_page);
        Self {
            client,
            query,
            current_page: page,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Fetch the next page of search results.
    ///
    /// This method handles pagination automatically and includes rate limiting
    /// to be respectful to Last.fm's servers.
    pub async fn next_page(&mut self) -> Result<Option<crate::ArtistPage>> {
        if !self.has_more {
            return Ok(None);
        }

        let page = self
            .client
            .search_artists_page(&self.query, self.current_page)
            .await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    /// Get the total number of pages, if known.
    ///
    /// Returns `None` until at least one page has been fetched.
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

// =============================================================================
// ARTISTS ITERATOR
// =============================================================================

/// Iterator for browsing all artists in the user's library.
///
/// This iterator provides access to all artists in the authenticated user's Last.fm library,
/// sorted by play count (highest first). The iterator loads artists as needed and handles
/// rate limiting automatically to be respectful to Last.fm's servers.
pub struct ArtistsIterator<C: LastFmEditClient> {
    client: C,
    current_page: u32,
    has_more: bool,
    buffer: Vec<crate::Artist>,
    total_pages: Option<u32>,
}

#[async_trait(?Send)]
impl<C: LastFmEditClient> AsyncPaginatedIterator<crate::Artist> for ArtistsIterator<C> {
    async fn next(&mut self) -> Result<Option<crate::Artist>> {
        // If buffer is empty, try to load next page
        if self.buffer.is_empty() {
            if let Some(page) = self.next_page().await? {
                self.buffer = page.artists;
                self.buffer.reverse(); // Reverse so we can pop from end efficiently
            }
        }

        Ok(self.buffer.pop())
    }

    fn current_page(&self) -> u32 {
        self.current_page.saturating_sub(1)
    }

    fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}

impl<C: LastFmEditClient> ArtistsIterator<C> {
    /// Create a new artists iterator.
    ///
    /// This iterator will start from page 1 and load all artists in the user's library.
    pub fn new(client: C) -> Self {
        Self {
            client,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Create a new artists iterator starting from a specific page.
    ///
    /// This is useful for implementing offset functionality efficiently by starting
    /// at the appropriate page rather than iterating through all previous pages.
    pub fn with_starting_page(client: C, starting_page: u32) -> Self {
        let page = std::cmp::max(1, starting_page);
        Self {
            client,
            current_page: page,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Fetch the next page of artists.
    ///
    /// This method handles pagination automatically and includes rate limiting
    /// to be respectful to Last.fm's servers.
    pub async fn next_page(&mut self) -> Result<Option<crate::ArtistPage>> {
        if !self.has_more {
            return Ok(None);
        }

        let page = self.client.get_artists_page(self.current_page).await?;

        self.has_more = page.has_next_page;
        self.current_page += 1;
        self.total_pages = page.total_pages;

        Ok(Some(page))
    }

    /// Get the total number of pages, if known.
    ///
    /// Returns `None` until at least one page has been fetched.
    pub fn total_pages(&self) -> Option<u32> {
        self.total_pages
    }
}
