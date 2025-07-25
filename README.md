# lastfm-edit

A Rust crate for programmatic access to Last.fm's scrobble editing functionality via web scraping.

This crate provides a high-level interface for authenticating with Last.fm, browsing user libraries,
and performing bulk edits on scrobbled tracks. It uses web scraping to access functionality not
available through Last.fm's public API.

## Features

- **Authentication**: Login to Last.fm with username/password
- **Library browsing**: Paginated access to tracks, albums, and recent scrobbles
- **Bulk editing**: Edit track names, artist names, and album information
- **Async iterators**: Stream large datasets efficiently
- **HTTP client abstraction**: Works with any HTTP client implementation

## Quick Start

```rust,no_run
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Create HTTP client and login
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::login_with_credentials(
        Box::new(http_client),
        "username",
        "password"
    ).await?;

    // Browse recent tracks
    let mut recent_tracks = client.recent_tracks();
    while let Some(track) = recent_tracks.next().await? {
        println!("{} - {}", track.artist, track.name);
    }

    Ok(())
}
```

## Core Components

- [`LastFmEditClient`] - **Main client trait** for interacting with Last.fm. **See the trait documentation for all available methods and detailed examples.**
- [`LastFmEditClientImpl`] - Concrete client implementation
- [`Track`], [`Album`] - Data structures for music metadata
- [`AsyncPaginatedIterator`] - Trait for streaming paginated data
- [`ScrobbleEdit`] - Represents track edit operations
- [`LastFmError`] - Error types for the crate

## Installation

Add this to your `Cargo.toml`:
```toml
[dependencies]
lastfm-edit = "3.0.0"
http-client = { version = "^6.6.3", package = "http-client-2", features = ["curl_client"] }
tokio = { version = "1.0", features = ["full"] }
```

## Documentation

📚 **For comprehensive API documentation and examples, see the [`LastFmEditClient`] trait documentation.**

The trait contains detailed documentation for all available methods including:
- Authentication and session management
- Library browsing (tracks, albums, recent scrobbles)
- Bulk editing operations
- Event monitoring for rate limiting
- Advanced usage patterns

## Usage Patterns

### Basic Library Browsing

```rust,no_run
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::login_with_credentials(
        Box::new(http_client),
        "username",
        "password"
    ).await?;

    // Get all tracks by an artist
    let mut tracks = client.artist_tracks("Radiohead");
    while let Some(track) = tracks.next().await? {
        println!("{} - {}", track.artist, track.name);
    }

    Ok(())
}
```

### Bulk Track Editing

```rust,no_run
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::login_with_credentials(
        Box::new(http_client),
        "username",
        "password"
    ).await?;

    // Find and edit tracks
    let tracks = client.artist_tracks("Artist Name").collect_all().await?;
    for track in tracks {
        if track.name.contains("(Remaster)") {
            let new_name = track.name.replace(" (Remaster)", "");

            // Use convenient edit methods from the trait
            let response = client.edit_artist_for_track(
                &track.name,
                &track.artist,
                &new_name
            ).await?;

            if response.success() {
                println!("Successfully edited: {} -> {}", track.name, new_name);
            }
        }
    }

    Ok(())
}
```

### Recent Tracks Monitoring

```rust,no_run
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::login_with_credentials(
        Box::new(http_client),
        "username",
        "password"
    ).await?;

    // Get recent tracks (first 100)
    let recent_tracks = client.recent_tracks().take(100).await?;
    println!("Found {} recent tracks", recent_tracks.len());

    Ok(())
}
```

## License

MIT