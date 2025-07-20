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

### 3. Paginated Track Iterator (`src/iterator.rs`)
- `ArtistTracksIterator` - custom iterator for paginated results
- Methods: `next_page()`, `collect_all()`, `take(n)`
- URL pattern: `{base_url}/user/{username}/library/music/{artist}/+tracks?page={n}`

### 4. HTML Parsing
- Parses track tables with selectors: `.chartlist`, `.chartlist-name a`, `.chartlist-count-bar-value`
- Pagination detection via `.pagination-list` and next page links
- Error handling for missing elements

### 5. Examples Structure
- `examples/login_test.rs` - Basic authentication test
- `examples/list_tracks.rs` - Simple track listing for an artist
- `examples/list_album_tracks.rs` - List editable tracks from a specific album
- `examples/rename_album.rs` - Album name editing example
- `examples/clean_artist_tracks.rs` - Generic track name cleanup with regex patterns
- `examples/tui.rs` - Interactive terminal UI for track editing
- Run: `cargo run --example login_test` or `cargo run --example list_tracks -- "Artist Name"`

## Key Dependencies
```toml
http-client = { version = "6.5", features = ["curl_client"] }
http-types = "2.12"
scraper = "0.20"
tokio = { version = "1.0", features = ["full"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "1.0"
urlencoding = "2.1"
```

## Next Steps
- Test `artist_tracks` example with real data
- Add more Last.fm scraping functionality as needed
- Implement scrobble editing once track listing is working

## Working Commands
```bash
# Basic authentication test
direnv exec . cargo run --example login_test

# Simple track listing
direnv exec . cargo run --example list_tracks -- "Artist Name"

# List editable tracks from an album
direnv exec . cargo run --example list_album_tracks -- "Artist Name" "Album Name"

# Album renaming
direnv exec . cargo run --example rename_album -- "Old Album Name" "New Album Name" "Artist Name"

# Track cleaning with regex patterns
direnv exec . cargo run --example clean_artist_tracks -- "Artist Name" " - Remastered( \\d{4})?$"

# Interactive TUI
direnv exec . cargo run --example tui -- "Artist Name"
```

## Reference Implementation
Original working implementation in `/home/imalison/Projects/scrobble-scrubber/src/client.rs` using reqwest - converted to http-client abstraction.