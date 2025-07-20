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
