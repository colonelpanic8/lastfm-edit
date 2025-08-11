# lastfm-edit

[![Crates.io](https://img.shields.io/crates/v/lastfm-edit.svg)](https://crates.io/crates/lastfm-edit)
[![Documentation](https://docs.rs/lastfm-edit/badge.svg)](https://docs.rs/lastfm-edit)
[![CI](https://github.com/colonelpanic8/lastfm-edit/actions/workflows/ci.yml/badge.svg)](https://github.com/colonelpanic8/lastfm-edit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A Rust crate for programmatic access to Last.fm's scrobble editing functionality via web scraping.

📚 **[View API Documentation →](https://docs.rs/lastfm-edit/latest/lastfm_edit/trait/trait.LastFmEditClient.html)**

## Features

- **Authentication**: Login with username/password
- **Library browsing**: Paginated access to tracks, albums, and recent scrobbles
- **Bulk editing**: Edit track names, artist names, and album information
- **Async iterators**: Stream large datasets efficiently
- **HTTP client abstraction**: Works with any HTTP client implementation

## Quick Start

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

    let mut recent_tracks = client.recent_tracks();
    while let Some(track) = recent_tracks.next().await? {
        println!("{} - {}", track.artist, track.name);
    }

    Ok(())
}
```

## Core Types

- [`LastFmEditClient`] - Main client trait (see trait docs for all methods and examples)
- [`LastFmEditClientImpl`] - Concrete client implementation
- [`Track`], [`Album`] - Music metadata structures
- [`AsyncPaginatedIterator`] - Streaming paginated data
- [`ScrobbleEdit`] - Track edit operations
- [`LastFmError`] - Error types

## Installation

```toml
[dependencies]
lastfm-edit = "4.0.1"
http-client = { version = "^6.6.3", package = "http-client-2", features = ["curl_client"] }
tokio = { version = "1.0", features = ["full"] }
```


## License

MIT
