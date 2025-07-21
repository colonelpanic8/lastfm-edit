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
use lastfm_edit::{LastFmEditClient, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Create client with any HTTP implementation
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmEditClient::new(Box::new(http_client));

    // Login to Last.fm
    client.login("username", "password").await?;

    // Browse recent tracks
    let mut recent_tracks = client.recent_tracks();
    while let Some(track) = recent_tracks.next().await? {
        println!("{} - {}", track.artist, track.name);
    }

    Ok(())
}
```

## Core Components

- [`LastFmClient`] - Main client for interacting with Last.fm
- [`Track`], [`Album`] - Data structures for music metadata
- [`AsyncPaginatedIterator`] - Trait for streaming paginated data
- [`ScrobbleEdit`] - Represents track edit operations
- [`LastFmError`] - Error types for the crate

## Installation

Add this to your `Cargo.toml`:
```toml
[dependencies]
lastfm-edit = "0.1.0"
http-client = { version = "6.5", features = ["curl_client"] }
tokio = { version = "1.0", features = ["full"] }
```

## Usage Patterns

### Basic Library Browsing

```rust,no_run
use lastfm_edit::{LastFmEditClient, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmEditClient::new(Box::new(http_client));

    client.login("username", "password").await?;

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
use lastfm_edit::{LastFmEditClient, ScrobbleEdit, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmEditClient::new(Box::new(http_client));

    client.login("username", "password").await?;

    // Find and edit tracks
    let tracks = client.artist_tracks("Artist Name").collect_all().await?;
    for track in tracks {
        if track.name.contains("(Remaster)") {
            let new_name = track.name.replace(" (Remaster)", "");

            // Create edit for this track
            let edit = ScrobbleEdit::from_track_info(
                &track.name,
                &track.name, // Use track name as album fallback
                &track.artist,
                0 // No timestamp needed for bulk edit
            )
            .with_track_name(&new_name)
            .with_edit_all(true);

            let response = client.edit_scrobble(&edit).await?;
            if response.success {
                println!("Successfully edited: {} -> {}", track.name, new_name);
            }
        }
    }

    Ok(())
}
```

### Recent Tracks Monitoring

```rust,no_run
use lastfm_edit::{LastFmEditClient, AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmEditClient::new(Box::new(http_client));

    client.login("username", "password").await?;

    // Get recent tracks (first 100)
    let recent_tracks = client.recent_tracks().take(100).await?;
    println!("Found {} recent tracks", recent_tracks.len());

    Ok(())
}
```

## License

MIT
