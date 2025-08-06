use super::list_output::{HumanReadableListHandler, JsonListHandler, ListEvent, ListOutputHandler};
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};

/// Handle the list artists command
pub async fn handle_list_artists(
    client: &LastFmEditClientImpl,
    limit: usize,
    verbose: bool,
    format: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create appropriate handler based on output format
    let mut handler: Box<dyn ListOutputHandler> = if json_output {
        Box::new(JsonListHandler::new())
    } else {
        Box::new(HumanReadableListHandler::new(verbose, format))
    };

    // Emit start event
    handler.handle_event(ListEvent::Started {
        command: "artists".to_string(),
        artist: None,
        album: None,
    });

    let mut artists_iterator = client.artists();
    let mut count = 0;

    while let Some(artist) = artists_iterator.next().await? {
        count += 1;

        // Emit artist found event
        handler.handle_event(ListEvent::ArtistFound {
            index: count,
            artist,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    // Emit summary event
    handler.handle_event(ListEvent::Summary {
        command: "artists".to_string(),
        total_items: count,
        artist: None,
        album: None,
    });

    Ok(())
}

/// Handle the list albums command
pub async fn handle_list_albums(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
    verbose: bool,
    format: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create appropriate handler based on output format
    let mut handler: Box<dyn ListOutputHandler> = if json_output {
        Box::new(JsonListHandler::new())
    } else {
        Box::new(HumanReadableListHandler::new(verbose, format))
    };

    // Emit start event
    handler.handle_event(ListEvent::Started {
        command: "albums".to_string(),
        artist: Some(artist.to_string()),
        album: None,
    });

    let mut albums_iterator = client.artist_albums(artist);
    let mut count = 0;

    while let Some(album) = albums_iterator.next().await? {
        count += 1;

        // Emit album found event
        handler.handle_event(ListEvent::AlbumFound {
            index: count,
            album,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    // Emit summary event
    handler.handle_event(ListEvent::Summary {
        command: "albums".to_string(),
        total_items: count,
        artist: Some(artist.to_string()),
        album: None,
    });

    Ok(())
}

/// Handle the list tracks by album command
pub async fn handle_list_tracks_by_album(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
    verbose: bool,
    format: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create appropriate handler based on output format
    let mut handler: Box<dyn ListOutputHandler> = if json_output {
        Box::new(JsonListHandler::new())
    } else {
        Box::new(HumanReadableListHandler::new(verbose, format))
    };

    // Emit start event
    handler.handle_event(ListEvent::Started {
        command: "tracks-by-album".to_string(),
        artist: Some(artist.to_string()),
        album: None,
    });

    let mut albums_iterator = client.artist_albums(artist);
    let mut album_count = 0;

    while let Some(album) = albums_iterator.next().await? {
        album_count += 1;

        // Emit album section event
        handler.handle_event(ListEvent::AlbumSection {
            album_index: album_count,
            album: album.clone(),
        });

        // Get tracks for this album
        let mut tracks_iterator = client.album_tracks(&album.name, artist);
        let mut track_idx = 0;
        let mut has_tracks = false;

        while let Some(track) = tracks_iterator.next().await.transpose() {
            match track {
                Ok(track) => {
                    has_tracks = true;
                    track_idx += 1;
                    handler.handle_event(ListEvent::AlbumTrackFound {
                        album_index: album_count,
                        track_index: track_idx,
                        track,
                    });
                }
                Err(e) => {
                    handler.handle_event(ListEvent::Error {
                        message: format!("Error getting tracks: {e}"),
                    });
                    break;
                }
            }
        }

        if !has_tracks {
            handler.handle_event(ListEvent::Error {
                message: "No tracks found in your library for this album".to_string(),
            });
        }

        if limit > 0 && album_count >= limit {
            break;
        }
    }

    // Emit summary event
    handler.handle_event(ListEvent::Summary {
        command: "tracks-by-album".to_string(),
        total_items: album_count,
        artist: Some(artist.to_string()),
        album: None,
    });

    Ok(())
}

/// Handle the list tracks command
pub async fn handle_list_tracks(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
    verbose: bool,
    format: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create appropriate handler based on output format
    let mut handler: Box<dyn ListOutputHandler> = if json_output {
        Box::new(JsonListHandler::new())
    } else {
        Box::new(HumanReadableListHandler::new(verbose, format))
    };

    // Emit start event
    handler.handle_event(ListEvent::Started {
        command: "tracks".to_string(),
        artist: Some(artist.to_string()),
        album: None,
    });

    let mut tracks_iterator = client.artist_tracks(artist);
    let mut count = 0;

    while let Some(track) = tracks_iterator.next().await? {
        count += 1;

        // Emit track found event
        handler.handle_event(ListEvent::TrackFound {
            index: count,
            track,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    // Emit summary event
    handler.handle_event(ListEvent::Summary {
        command: "tracks".to_string(),
        total_items: count,
        artist: Some(artist.to_string()),
        album: None,
    });

    Ok(())
}

/// Handle the list tracks direct command
pub async fn handle_list_tracks_direct(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
    verbose: bool,
    format: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create appropriate handler based on output format
    let mut handler: Box<dyn ListOutputHandler> = if json_output {
        Box::new(JsonListHandler::new())
    } else {
        Box::new(HumanReadableListHandler::new(verbose, format))
    };

    // Emit start event
    handler.handle_event(ListEvent::Started {
        command: "tracks-direct".to_string(),
        artist: Some(artist.to_string()),
        album: None,
    });

    let mut tracks_iterator = client.artist_tracks_direct(artist);
    let mut count = 0;

    while let Some(track) = tracks_iterator.next().await? {
        count += 1;

        // Emit track found event
        handler.handle_event(ListEvent::TrackFound {
            index: count,
            track,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    // Emit summary event
    handler.handle_event(ListEvent::Summary {
        command: "tracks-direct".to_string(),
        total_items: count,
        artist: Some(artist.to_string()),
        album: None,
    });

    Ok(())
}

/// Handle the list album tracks command
pub async fn handle_list_album_tracks(
    client: &LastFmEditClientImpl,
    album: &str,
    artist: &str,
    details: bool,
    format: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Create appropriate handler based on output format
    let mut handler: Box<dyn ListOutputHandler> = if json_output {
        Box::new(JsonListHandler::new())
    } else {
        Box::new(HumanReadableListHandler::new(details, format))
    };

    // Emit start event
    handler.handle_event(ListEvent::Started {
        command: "album-tracks".to_string(),
        artist: Some(artist.to_string()),
        album: Some(album.to_string()),
    });

    let mut tracks_iterator = client.album_tracks(album, artist);
    let mut count = 0;

    while let Some(track) = tracks_iterator.next().await? {
        count += 1;

        // Emit track found event
        handler.handle_event(ListEvent::TrackFound {
            index: count,
            track,
        });
    }

    // Emit summary event
    handler.handle_event(ListEvent::Summary {
        command: "album-tracks".to_string(),
        total_items: count,
        artist: Some(artist.to_string()),
        album: Some(album.to_string()),
    });

    Ok(())
}
