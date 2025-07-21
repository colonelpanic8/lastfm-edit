use crate::{Album, AlbumPage, LastFmEditClient, Result, Track, TrackPage};

/// Async iterator trait for paginated Last.fm data.
///
/// This trait provides a common interface for iterating over paginated data from Last.fm,
/// such as tracks, albums, and recent scrobbles. All iterators implement efficient streaming
/// with automatic pagination and built-in rate limiting.
///
/// # Examples
///
/// ```rust,no_run
/// use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
///
/// # tokio_test::block_on(async {
/// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
/// // client.login(...).await?;
///
/// let mut tracks = client.artist_tracks("Radiohead");
///
/// // Iterate one by one
/// while let Some(track) = tracks.next().await? {
///     println!("{}", track.name);
/// }
///
/// // Or collect a limited number
/// let first_10 = tracks.take(10).await?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
#[allow(async_fn_in_trait)]
pub trait AsyncPaginatedIterator {
    /// The item type yielded by this iterator
    type Item;

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
    async fn next(&mut self) -> Result<Option<Self::Item>>;

    /// Collect all remaining items into a Vec.
    ///
    /// **Warning**: This method will fetch ALL remaining pages, which could be
    /// many thousands of items for large libraries. Use [`take`](Self::take) for
    /// safer bounded collection.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
    /// # tokio_test::block_on(async {
    /// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
    /// let mut tracks = client.artist_tracks("Small Artist");
    /// let all_tracks = tracks.collect_all().await?;
    /// println!("Found {} tracks total", all_tracks.len());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    async fn collect_all(&mut self) -> Result<Vec<Self::Item>> {
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
    /// # tokio_test::block_on(async {
    /// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
    /// let mut tracks = client.artist_tracks("Radiohead");
    /// let top_20 = tracks.take(20).await?;
    /// println!("Top 20 tracks: {:?}", top_20);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
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

    /// Get the current page number (0-indexed).
    ///
    /// Returns the page number of the most recently fetched page.
    fn current_page(&self) -> u32;
}

/// Iterator for browsing an artist's tracks from a user's library.
///
/// This iterator provides paginated access to all tracks by a specific artist
/// in the authenticated user's Last.fm library, ordered by play count.
///
/// # Examples
///
/// ```rust,no_run
/// # use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
/// # tokio_test::block_on(async {
/// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
/// // client.login(...).await?;
///
/// let mut tracks = client.artist_tracks("The Beatles");
///
/// // Get the top 5 most played tracks
/// let top_tracks = tracks.take(5).await?;
/// for track in top_tracks {
///     println!("{} (played {} times)", track.name, track.playcount);
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub struct ArtistTracksIterator<'a> {
    client: &'a mut LastFmEditClient,
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
    /// Create a new artist tracks iterator.
    ///
    /// This is typically called via [`LastFmClient::artist_tracks`](crate::LastFmClient::artist_tracks).
    pub fn new(client: &'a mut LastFmEditClient, artist: String) -> Self {
        Self {
            client,
            artist,
            current_page: 1,
            has_more: true,
            buffer: Vec::new(),
            total_pages: None,
        }
    }

    /// Fetch the next page of tracks.
    ///
    /// This method handles pagination automatically and includes rate limiting
    /// to be respectful to Last.fm's servers.
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
///
/// # Examples
///
/// ```rust,no_run
/// # use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
/// # tokio_test::block_on(async {
/// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
/// // client.login(...).await?;
///
/// let mut albums = client.artist_albums("Pink Floyd");
///
/// // Get all albums (be careful with large discographies!)
/// while let Some(album) = albums.next().await? {
///     println!("{} (played {} times)", album.name, album.playcount);
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub struct ArtistAlbumsIterator<'a> {
    client: &'a mut LastFmEditClient,
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
    /// Create a new artist albums iterator.
    ///
    /// This is typically called via [`LastFmClient::artist_albums`](crate::LastFmClient::artist_albums).
    pub fn new(client: &'a mut LastFmEditClient, artist: String) -> Self {
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
///
/// # Examples
///
/// ```rust,no_run
/// # use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
/// # tokio_test::block_on(async {
/// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
/// // client.login(...).await?;
///
/// // Get recent tracks with timestamps
/// let mut recent = client.recent_tracks();
/// while let Some(track) = recent.next().await? {
///     if let Some(timestamp) = track.timestamp {
///         println!("{} - {} ({})", track.artist, track.name, timestamp);
///     }
/// }
///
/// // Or stop at a specific timestamp to avoid reprocessing
/// let last_processed = 1640995200;
/// let mut recent = client.recent_tracks().with_stop_timestamp(last_processed);
/// let new_tracks = recent.collect_all().await?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub struct RecentTracksIterator<'a> {
    client: &'a mut LastFmEditClient,
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
    /// Create a new recent tracks iterator.
    ///
    /// This is typically called via [`LastFmClient::recent_tracks`](crate::LastFmClient::recent_tracks).
    pub fn new(client: &'a mut LastFmEditClient) -> Self {
        Self {
            client,
            current_page: 1,
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
    /// # tokio_test::block_on(async {
    /// let mut client = LastFmClient::new(Box::new(http_client::native::NativeClient::new()));
    /// let last_processed = 1640995200; // Some previous timestamp
    ///
    /// let mut recent = client.recent_tracks().with_stop_timestamp(last_processed);
    /// let new_tracks = recent.collect_all().await?; // Only gets new tracks
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    pub fn with_stop_timestamp(mut self, timestamp: u64) -> Self {
        self.stop_at_timestamp = Some(timestamp);
        self
    }
}
