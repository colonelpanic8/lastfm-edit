# lastfm-edit Rust Crate

## Project Overview
Building a Rust crate for programmatic access to Last.fm's scrobble editing functionality via web scraping using the `http-client` abstraction library.

## Environment
- Uses direnv with nix flake for development environment
- Environment variables: `LASTFM_EDIT_USERNAME`, `LASTFM_EDIT_PASSWORD` set in `.envrc`
- Run with: `direnv exec . cargo run --example <name>`

## Current Status ✅
Successfully implemented:

### 1. Login System (`src/client.rs`)
- `LastFmClient::new(http_client)` - accepts any HttpClient implementation
- `LastFmClient::with_base_url()` - configurable base URL
- `login()` - scrapes CSRF tokens, submits login forms
- Working authentication with Last.fm

### 2. Track Data Structures (`src/track.rs`)
```rust
pub struct Track {
    pub name: String,
    pub artist: String,
    pub playcount: u32,
}

pub struct TrackPage {
    pub tracks: Vec<Track>,
    pub page_number: u32,
    pub has_next_page: bool,
    pub total_pages: Option<u32>,
}
```

## Future Extension: Automated Track Monitoring & Scrubbing System

### Requirements & Design Notes

#### 1. Recent Tracks API Enhancement
- Add new functionality to `src/client.rs` for listing recent tracks in paginated way
- Should integrate with existing iterator system for consistency
- Need paginated access to user's recently played tracks

#### 2. State Management & Stopping Criteria
- Need to maintain state about "what we have already seen/processed"
- Determine when to stop reading recent tracks based on previously processed tracks
- Logic for deciding how far back to read in recently played tracks pagination
- Implement method/function to read until a specific timestamp
- Alternative: identify most recent scrobble that we have already seen/processed

#### 3. Track Processing System
- Process new tracks (possibly asynchronously)
- Apply concrete rules system for automated scrubbing
- MCP integration for LLM-based track analysis and rewrites (needs research - unclear if good fit)

## Implementation Status ✅

### Recent Tracks Iterator System
- ✅ Created `RecentTracksIterator` with consistent API pattern
- ✅ Implemented shared `AsyncPaginatedIterator` trait with associated type `Item`
- ✅ All iterators (`ArtistTracksIterator`, `ArtistAlbumsIterator`, `RecentTracksIterator`) implement the trait
- ✅ Support for timestamp-based stopping criteria via `with_stop_timestamp()`
- ✅ Added `client.recent_tracks()` method for easy access
- ✅ Example: `examples/list_recent_tracks.rs` with command-line parameter support

### Key Features
- **Consistent API**: All iterators share `next()`, `collect_all()`, `take()`, `current_page()` methods
- **Timestamp filtering**: Recent tracks iterator can stop at a specific timestamp to avoid reprocessing
- **Streaming output**: Example shows tracks as they're fetched, not batched
- **Configurable limits**: Command-line argument controls number of tracks to fetch
- **Fixed parsing**: Now correctly parses all chartlist tables (50 tracks per page instead of ~27)

### Bug Fixes ✅
- **Recent tracks parsing**: Fixed to process all `table.chartlist` elements, not just the first one
- **Test coverage**: Added `tests/recent_tracks_parsing_test.rs` to verify 50 tracks per page
- **Debug capabilities**: Made `parse_recent_scrobbles()` public for testing
