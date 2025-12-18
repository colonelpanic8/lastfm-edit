//! HTML parsing utilities for Last.fm pages.
//!
//! This module contains all the HTML parsing logic for extracting track, album,
//! and other data from Last.fm web pages. These functions are primarily pure
//! functions that take HTML documents and return structured data.

use crate::{Album, AlbumPage, Artist, ArtistPage, LastFmError, Result, Track, TrackPage};
use scraper::{Html, Selector};

/// Parser struct containing parsing methods for Last.fm HTML pages.
///
/// This struct holds the parsing logic that was previously embedded in the client.
/// It's designed to be stateless and focused purely on HTML parsing.
#[derive(Debug, Clone)]
pub struct LastFmParser;

impl LastFmParser {
    /// Create a new parser instance.
    pub fn new() -> Self {
        Self
    }

    /// Parse recent scrobbles from the user's library page
    /// This extracts real scrobble data with timestamps for editing
    pub fn parse_recent_scrobbles(&self, document: &Html) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();

        // Recent scrobbles are typically in chartlist tables - there can be multiple
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tbody tr").unwrap();

        let tables: Vec<_> = document.select(&table_selector).collect();
        log::debug!("Found {} chartlist tables", tables.len());

        for table in tables {
            for row in table.select(&row_selector) {
                if let Ok(track) = self.parse_recent_scrobble_row(&row) {
                    tracks.push(track);
                }
            }
        }

        if tracks.is_empty() {
            log::debug!("No tracks found in recent scrobbles");
        }

        log::debug!("Parsed {} recent scrobbles", tracks.len());
        Ok(tracks)
    }

    /// Parse a single row from the recent scrobbles table
    fn parse_recent_scrobble_row(&self, row: &scraper::ElementRef) -> Result<Track> {
        // Extract track name
        let name_selector = Selector::parse(".chartlist-name a").unwrap();
        let name = row
            .select(&name_selector)
            .next()
            .ok_or(LastFmError::Parse("Missing track name".to_string()))?
            .text()
            .collect::<String>()
            .trim()
            .to_string();

        // Extract artist name
        let artist_selector = Selector::parse(".chartlist-artist a").unwrap();
        let artist = row
            .select(&artist_selector)
            .next()
            .ok_or(LastFmError::Parse("Missing artist name".to_string()))?
            .text()
            .collect::<String>()
            .trim()
            .to_string();

        // Extract timestamp from data attributes or hidden inputs
        let timestamp = self.extract_scrobble_timestamp(row);

        // Extract album from hidden inputs in edit form
        let album = self.extract_scrobble_album(row);

        // Extract album artist from hidden inputs in edit form
        let album_artist = self.extract_scrobble_album_artist(row);

        // For recent scrobbles, playcount is typically 1 since they're individual scrobbles
        let playcount = 1;

        Ok(Track {
            name,
            artist,
            playcount,
            timestamp,
            album,
            album_artist,
        })
    }

    /// Extract timestamp from scrobble row elements
    fn extract_scrobble_timestamp(&self, row: &scraper::ElementRef) -> Option<u64> {
        // Look for timestamp in various places:

        // 1. Check for data-timestamp attribute
        if let Some(timestamp_str) = row.value().attr("data-timestamp") {
            if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                return Some(timestamp);
            }
        }

        // 2. Look for hidden timestamp input
        let timestamp_input_selector = Selector::parse("input[name='timestamp']").unwrap();
        if let Some(input) = row.select(&timestamp_input_selector).next() {
            if let Some(value) = input.value().attr("value") {
                if let Ok(timestamp) = value.parse::<u64>() {
                    return Some(timestamp);
                }
            }
        }

        // 3. Look for edit form with timestamp
        let edit_form_selector =
            Selector::parse("form[data-edit-scrobble] input[name='timestamp']").unwrap();
        if let Some(timestamp_input) = row.select(&edit_form_selector).next() {
            if let Some(value) = timestamp_input.value().attr("value") {
                if let Ok(timestamp) = value.parse::<u64>() {
                    return Some(timestamp);
                }
            }
        }

        // Removed time element parsing - testing if needed

        None
    }

    /// Extract album name from scrobble row elements
    fn extract_scrobble_album(&self, row: &scraper::ElementRef) -> Option<String> {
        // Look for album_name in hidden inputs within edit forms
        let album_input_selector =
            Selector::parse("form[data-edit-scrobble] input[name='album_name']").unwrap();

        if let Some(album_input) = row.select(&album_input_selector).next() {
            if let Some(album_name) = album_input.value().attr("value") {
                if !album_name.is_empty() {
                    return Some(album_name.to_string());
                }
            }
        }

        None
    }

    /// Extract album artist name from scrobble row elements
    fn extract_scrobble_album_artist(&self, row: &scraper::ElementRef) -> Option<String> {
        // Look for album_artist_name in hidden inputs within edit forms
        let album_artist_input_selector =
            Selector::parse("form[data-edit-scrobble] input[name='album_artist_name']").unwrap();

        if let Some(album_artist_input) = row.select(&album_artist_input_selector).next() {
            if let Some(album_artist_name) = album_artist_input.value().attr("value") {
                if !album_artist_name.is_empty() {
                    return Some(album_artist_name.to_string());
                }
            }
        }

        None
    }

    /// Parse a tracks page into a `TrackPage` structure
    pub fn parse_tracks_page(
        &self,
        document: &Html,
        page_number: u32,
        artist: &str,
        album: Option<&str>,
    ) -> Result<TrackPage> {
        let tracks = self.extract_tracks_from_document(document, artist, album)?;

        // Check for pagination
        let (has_next_page, total_pages) = self.parse_pagination(document, page_number)?;

        Ok(TrackPage {
            tracks,
            page_number,
            has_next_page,
            total_pages,
        })
    }

    /// Extract tracks from HTML document
    pub fn extract_tracks_from_document(
        &self,
        document: &Html,
        artist: &str,
        album: Option<&str>,
    ) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        let mut seen_tracks = std::collections::HashSet::new();

        log::debug!("Starting track extraction for artist: {artist}, album: {album:?}");

        // JSON parsing removed - was not implemented and always failed

        // Parse track data from data-track-name attributes (AJAX response)
        let track_selector = Selector::parse("[data-track-name]").unwrap();
        let track_elements: Vec<_> = document.select(&track_selector).collect();
        log::debug!(
            "Found {} elements with data-track-name",
            track_elements.len()
        );

        for element in track_elements {
            let track_name = element.value().attr("data-track-name").unwrap_or("");
            if track_name.is_empty() {
                continue;
            }
            if seen_tracks.contains(track_name) {
                continue;
            }
            seen_tracks.insert(track_name.to_string());

            match self.find_playcount_for_track(document, track_name) {
                Ok(playcount) => {
                    let timestamp = self.find_timestamp_for_track(document, track_name);
                    let track = Track {
                        name: track_name.to_string(),
                        artist: artist.to_string(),
                        playcount,
                        timestamp,
                        album: album.map(|a| a.to_string()),
                        album_artist: None, // Not available in aggregate track listings
                    };
                    tracks.push(track);
                    log::debug!("Added track '{track_name}' with {playcount} plays");
                }
                Err(e) => {
                    log::debug!("FAILED to find playcount for track '{track_name}': {e}");
                }
            }
        }

        // Always try fallback parsing from chartlist tables to catch tracks without data-track-name
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let tables: Vec<_> = document.select(&table_selector).collect();

        for table in tables {
            let row_selector = Selector::parse("tbody tr").unwrap();
            let rows: Vec<_> = table.select(&row_selector).collect();

            for row in rows.iter() {
                // Try to parse as track row
                if let Ok(mut track) = self.parse_track_row(row) {
                    track.artist = artist.to_string();
                    if let Some(album_name) = album {
                        track.album = Some(album_name.to_string());
                    }

                    // Only add if we don't already have this track
                    if !seen_tracks.contains(&track.name) {
                        seen_tracks.insert(track.name.clone());
                        tracks.push(track);
                    }
                }
            }
        }

        log::debug!("Successfully extracted {} unique tracks", tracks.len());
        Ok(tracks)
    }

    // Removed parse_tracks_from_rows - no longer needed

    /// Parse a single track row from chartlist table
    pub fn parse_track_row(&self, row: &scraper::ElementRef) -> Result<Track> {
        // Extract track name using shared method
        let name = self.extract_name_from_row(row, "track")?;

        // Parse play count using shared method
        let playcount = self.extract_playcount_from_row(row);

        let artist = "".to_string(); // Will be filled in by caller

        Ok(Track {
            name,
            artist,
            playcount,
            timestamp: None,    // Not available in table parsing mode
            album: None,        // Not available in table parsing mode
            album_artist: None, // Not available in table parsing mode
        })
    }

    /// Parse albums page into `AlbumPage` structure
    pub fn parse_albums_page(
        &self,
        document: &Html,
        page_number: u32,
        artist: &str,
    ) -> Result<AlbumPage> {
        let mut albums = Vec::new();

        // Try parsing album data from data attributes (AJAX response)
        let album_selector = Selector::parse("[data-album-name]").unwrap();
        let album_elements: Vec<_> = document.select(&album_selector).collect();

        if !album_elements.is_empty() {
            log::debug!(
                "Found {} album elements with data-album-name",
                album_elements.len()
            );

            // Use a set to track unique albums
            let mut seen_albums = std::collections::HashSet::new();

            for element in album_elements {
                let album_name = element.value().attr("data-album-name").unwrap_or("");
                if !album_name.is_empty() && !seen_albums.contains(album_name) {
                    seen_albums.insert(album_name.to_string());

                    if let Ok(playcount) = self.find_playcount_for_album(document, album_name) {
                        let timestamp = self.find_timestamp_for_album(document, album_name);
                        let album = Album {
                            name: album_name.to_string(),
                            artist: artist.to_string(),
                            playcount,
                            timestamp,
                        };
                        albums.push(album);
                    }
                }
            }
        } else {
            // Fall back to parsing album rows from chartlist tables
            albums = self.parse_albums_from_rows(document, artist)?;
        }

        let (has_next_page, total_pages) = self.parse_pagination(document, page_number)?;

        Ok(AlbumPage {
            albums,
            page_number,
            has_next_page,
            total_pages,
        })
    }

    /// Parse albums from chartlist table rows
    fn parse_albums_from_rows(&self, document: &Html, artist: &str) -> Result<Vec<Album>> {
        let mut albums = Vec::new();
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tbody tr").unwrap();

        for table in document.select(&table_selector) {
            for row in table.select(&row_selector) {
                if let Ok(mut album) = self.parse_album_row(&row) {
                    album.artist = artist.to_string();
                    albums.push(album);
                }
            }
        }
        Ok(albums)
    }

    /// Parse a single album row from chartlist table
    pub fn parse_album_row(&self, row: &scraper::ElementRef) -> Result<Album> {
        // Extract album name using shared method
        let name = self.extract_name_from_row(row, "album")?;

        // Parse play count using shared method
        let playcount = self.extract_playcount_from_row(row);

        let artist = "".to_string(); // Will be filled in by caller

        Ok(Album {
            name,
            artist,
            playcount,
            timestamp: None, // Not available in table parsing
        })
    }

    // === SEARCH RESULTS PARSING ===

    /// Parse track search results from AJAX response
    ///
    /// This parses the HTML returned by `/user/{username}/library/tracks/search?ajax=1&query={query}`
    /// which contains chartlist tables with track results.
    pub fn parse_track_search_results(&self, document: &Html) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();

        // Search results use the same chartlist structure as library pages
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tbody tr").unwrap();

        let tables: Vec<_> = document.select(&table_selector).collect();
        log::debug!("Found {} chartlist tables in search results", tables.len());

        for table in tables {
            for row in table.select(&row_selector) {
                if let Ok(track) = self.parse_search_track_row(&row) {
                    tracks.push(track);
                }
            }
        }

        log::debug!("Parsed {} tracks from search results", tracks.len());
        Ok(tracks)
    }

    /// Parse album search results from AJAX response
    ///
    /// This parses the HTML returned by `/user/{username}/library/albums/search?ajax=1&query={query}`
    /// which contains chartlist tables with album results.
    pub fn parse_album_search_results(&self, document: &Html) -> Result<Vec<Album>> {
        let mut albums = Vec::new();

        // Search results use the same chartlist structure as library pages
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tbody tr").unwrap();

        let tables: Vec<_> = document.select(&table_selector).collect();
        log::debug!(
            "Found {} chartlist tables in album search results",
            tables.len()
        );

        for table in tables {
            for row in table.select(&row_selector) {
                if let Ok(album) = self.parse_search_album_row(&row) {
                    albums.push(album);
                }
            }
        }

        log::debug!("Parsed {} albums from search results", albums.len());
        Ok(albums)
    }

    /// Parse artist search results from AJAX response
    ///
    /// This parses the HTML returned by `/user/{username}/library/artists/search?ajax=1&query={query}`
    /// which contains chartlist tables with artist results.
    pub fn parse_artist_search_results(&self, document: &Html) -> Result<Vec<Artist>> {
        let mut artists = Vec::new();

        // Search results use the same chartlist structure as library pages
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tbody tr").unwrap();

        let tables: Vec<_> = document.select(&table_selector).collect();
        log::debug!(
            "Found {} chartlist tables in artist search results",
            tables.len()
        );

        for table in tables {
            for row in table.select(&row_selector) {
                if let Ok(artist) = self.parse_search_artist_row(&row) {
                    artists.push(artist);
                }
            }
        }

        log::debug!("Parsed {} artists from search results", artists.len());
        Ok(artists)
    }

    /// Parse a single artist row from search results
    fn parse_search_artist_row(&self, row: &scraper::ElementRef) -> Result<Artist> {
        // Extract artist name from the name column
        let name_selector = Selector::parse("td.chartlist-name a").unwrap();
        let name = row
            .select(&name_selector)
            .next()
            .ok_or(LastFmError::Parse(
                "Missing artist name in search results".to_string(),
            ))?
            .text()
            .collect::<String>()
            .trim()
            .to_string();

        // Extract playcount from the count bar
        let playcount = self.extract_playcount_from_row(row);

        Ok(Artist {
            name,
            playcount,
            timestamp: None, // Search results don't have timestamps
        })
    }

    /// Parse a single track row from search results
    fn parse_search_track_row(&self, row: &scraper::ElementRef) -> Result<Track> {
        // Extract track name using the standard chartlist structure
        let name = self.extract_name_from_row(row, "track")?;

        // Extract artist name from chartlist-artist column
        let artist_selector = Selector::parse(".chartlist-artist a").unwrap();
        let artist = row
            .select(&artist_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .ok_or_else(|| {
                LastFmError::Parse("Missing artist name in search results".to_string())
            })?;

        // Extract playcount from the bar value
        let playcount = self.extract_playcount_from_row(row);

        // Search results typically don't have timestamps since they're aggregated
        let timestamp = None;

        // Try to extract album information if available in the search results
        let album = self.extract_album_from_search_row(row);
        let album_artist = self.extract_album_artist_from_search_row(row);

        Ok(Track {
            name,
            artist,
            playcount,
            timestamp,
            album,
            album_artist,
        })
    }

    /// Parse a single album row from search results
    fn parse_search_album_row(&self, row: &scraper::ElementRef) -> Result<Album> {
        // Extract album name using the standard chartlist structure
        let name = self.extract_name_from_row(row, "album")?;

        // Extract artist name from chartlist-artist column
        let artist_selector = Selector::parse(".chartlist-artist a").unwrap();
        let artist = row
            .select(&artist_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .ok_or_else(|| {
                LastFmError::Parse("Missing artist name in album search results".to_string())
            })?;

        // Extract playcount from the bar value
        let playcount = self.extract_playcount_from_row(row);

        Ok(Album {
            name,
            artist,
            playcount,
            timestamp: None, // Search results don't have timestamps
        })
    }

    /// Extract album information from search track row
    fn extract_album_from_search_row(&self, row: &scraper::ElementRef) -> Option<String> {
        // Look for album information in hidden form inputs (similar to recent scrobbles)
        let album_input_selector = Selector::parse("input[name='album']").unwrap();
        if let Some(input) = row.select(&album_input_selector).next() {
            if let Some(value) = input.value().attr("value") {
                let album = value.trim().to_string();
                if !album.is_empty() {
                    return Some(album);
                }
            }
        }
        None
    }

    /// Extract album artist information from search track row
    fn extract_album_artist_from_search_row(&self, row: &scraper::ElementRef) -> Option<String> {
        // Look for album artist information in hidden form inputs
        let album_artist_input_selector = Selector::parse("input[name='album_artist']").unwrap();
        if let Some(input) = row.select(&album_artist_input_selector).next() {
            if let Some(value) = input.value().attr("value") {
                let album_artist = value.trim().to_string();
                if !album_artist.is_empty() {
                    return Some(album_artist);
                }
            }
        }
        None
    }

    // === SHARED PARSING UTILITIES ===

    /// Extract name from chartlist row (works for both tracks and albums)
    fn extract_name_from_row(&self, row: &scraper::ElementRef, item_type: &str) -> Result<String> {
        let name_selector = Selector::parse(".chartlist-name a").unwrap();
        let name = row
            .select(&name_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .ok_or_else(|| LastFmError::Parse(format!("Missing {item_type} name")))?;
        Ok(name)
    }

    /// Extract playcount from chartlist row (works for both tracks and albums)
    fn extract_playcount_from_row(&self, row: &scraper::ElementRef) -> u32 {
        let playcount_selector = Selector::parse(".chartlist-count-bar-value").unwrap();
        let mut playcount = 1; // default fallback

        if let Some(element) = row.select(&playcount_selector).next() {
            let text = element.text().collect::<String>().trim().to_string();
            // Extract just the number part (before "scrobbles" if present)
            if let Some(number_part) = text.split_whitespace().next() {
                if let Ok(count) = number_part.parse::<u32>() {
                    playcount = count;
                }
            }
        }
        playcount
    }

    /// Parse pagination information from document
    pub fn parse_pagination(
        &self,
        document: &Html,
        _current_page: u32,
    ) -> Result<(bool, Option<u32>)> {
        let pagination_selector = Selector::parse(".pagination-list").unwrap();

        if let Some(pagination) = document.select(&pagination_selector).next() {
            // Try multiple possible selectors for next page link
            let next_selectors = [
                "a[aria-label=\"Next\"]",
                ".pagination-next a",
                "a:contains(\"Next\")",
                ".next a",
            ];

            let mut has_next = false;
            for selector_str in &next_selectors {
                if let Ok(selector) = Selector::parse(selector_str) {
                    if pagination.select(&selector).next().is_some() {
                        has_next = true;
                        break;
                    }
                }
            }

            // Try to extract total pages from pagination text
            let total_pages = self.extract_total_pages_from_pagination(&pagination);

            Ok((has_next, total_pages))
        } else {
            // No pagination found - single page
            Ok((false, Some(1)))
        }
    }

    /// Helper functions for pagination parsing
    fn extract_total_pages_from_pagination(&self, pagination: &scraper::ElementRef) -> Option<u32> {
        // Look for patterns like "Page 1 of 42"
        let text = pagination.text().collect::<String>();
        if let Some(of_pos) = text.find(" of ") {
            let after_of = &text[of_pos + 4..];
            if let Some(number_end) = after_of.find(|c: char| !c.is_ascii_digit()) {
                if let Ok(total) = after_of[..number_end].parse::<u32>() {
                    return Some(total);
                }
            } else if let Ok(total) = after_of.trim().parse::<u32>() {
                return Some(total);
            }
        }
        None
    }

    // === JSON PARSING METHODS ===
    // Removed unused JSON parsing method

    // === FIND HELPER METHODS ===

    pub fn find_timestamp_for_track(&self, _document: &Html, _track_name: &str) -> Option<u64> {
        // Implementation would search for timestamp data
        None
    }

    pub fn find_playcount_for_track(&self, document: &Html, track_name: &str) -> Result<u32> {
        // Look for chartlist-count-bar-value elements near the track
        let count_selector = Selector::parse(".chartlist-count-bar-value").unwrap();
        let link_selector = Selector::parse("a[href*=\"/music/\"]").unwrap();

        // Find all track links that match our track name
        for link in document.select(&link_selector) {
            let link_text = link.text().collect::<String>().trim().to_string();
            if link_text == track_name {
                if let Some(row) = self.find_ancestor_row(link) {
                    if let Some(count_element) = row.select(&count_selector).next() {
                        let text = count_element.text().collect::<String>().trim().to_string();
                        if let Some(number_part) = text.split_whitespace().next() {
                            if let Ok(count) = number_part.parse::<u32>() {
                                return Ok(count);
                            }
                        }
                    }
                }
            }
        }
        Err(LastFmError::Parse(format!(
            "Could not find playcount for track: {track_name}"
        )))
    }

    pub fn find_timestamp_for_album(&self, _document: &Html, _album_name: &str) -> Option<u64> {
        // Implementation would search for timestamp data
        None
    }

    pub fn find_playcount_for_album(&self, document: &Html, album_name: &str) -> Result<u32> {
        // Look for chartlist-count-bar-value elements near the album
        let count_selector = Selector::parse(".chartlist-count-bar-value").unwrap();
        let link_selector = Selector::parse("a[href*=\"/music/\"]").unwrap();

        // Find all album links that match our album name
        for link in document.select(&link_selector) {
            let link_text = link.text().collect::<String>().trim().to_string();
            if link_text == album_name {
                if let Some(row) = self.find_ancestor_row(link) {
                    if let Some(count_element) = row.select(&count_selector).next() {
                        let text = count_element.text().collect::<String>().trim().to_string();
                        if let Some(number_part) = text.split_whitespace().next() {
                            if let Ok(count) = number_part.parse::<u32>() {
                                return Ok(count);
                            }
                        }
                    }
                }
            }
        }
        Err(LastFmError::Parse(format!(
            "Could not find playcount for album: {album_name}"
        )))
    }

    pub fn find_ancestor_row<'a>(
        &self,
        element: scraper::ElementRef<'a>,
    ) -> Option<scraper::ElementRef<'a>> {
        let mut current = element;
        while let Some(parent) = current.parent() {
            if let Some(parent_elem) = scraper::ElementRef::wrap(parent) {
                if parent_elem.value().name() == "tr" {
                    return Some(parent_elem);
                }
                current = parent_elem;
            } else {
                break;
            }
        }
        None
    }

    /// Parse artists page from user's library
    pub fn parse_artists_page(&self, document: &Html, page_number: u32) -> Result<ArtistPage> {
        let mut artists = Vec::new();

        // Parse artists from chartlist table rows
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tr.js-link-block").unwrap();

        let tables: Vec<_> = document.select(&table_selector).collect();
        log::debug!("Found {} chartlist tables for artists", tables.len());

        for table in tables {
            for row in table.select(&row_selector) {
                if let Ok(artist) = self.parse_artist_row(&row) {
                    artists.push(artist);
                }
            }
        }

        log::debug!("Parsed {} artists from page {}", artists.len(), page_number);

        let (has_next_page, total_pages) = self.parse_pagination(document, page_number)?;

        Ok(ArtistPage {
            artists,
            page_number,
            has_next_page,
            total_pages,
        })
    }

    /// Parse a single artist row from the artist library table
    fn parse_artist_row(&self, row: &scraper::ElementRef) -> Result<Artist> {
        // Extract artist name from the name column
        let name_selector = Selector::parse("td.chartlist-name a").unwrap();
        let name = row
            .select(&name_selector)
            .next()
            .ok_or(LastFmError::Parse("Missing artist name".to_string()))?
            .text()
            .collect::<String>()
            .trim()
            .to_string();

        // Extract playcount from the count bar
        let count_selector = Selector::parse(".chartlist-count-bar").unwrap();
        let playcount = if let Some(count_element) = row.select(&count_selector).next() {
            let count_text = count_element.text().collect::<String>();
            self.extract_number_from_count_text(&count_text)
                .unwrap_or(0)
        } else {
            0
        };

        // Artists in library listings typically don't have individual timestamps
        let timestamp = None;

        Ok(Artist {
            name,
            playcount,
            timestamp,
        })
    }

    /// Extract numeric value from count text like "3,395 scrobbles"
    fn extract_number_from_count_text(&self, text: &str) -> Option<u32> {
        // Remove commas and extract the first numeric part
        let cleaned = text.replace(',', "");
        cleaned.split_whitespace().next()?.parse::<u32>().ok()
    }
}

impl Default for LastFmParser {
    fn default() -> Self {
        Self::new()
    }
}
