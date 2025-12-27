use super::list_output::{log_started, log_summary, output_event, ListEvent};
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};

/// Handle the list artists command
pub async fn handle_list_artists(
    client: &LastFmEditClientImpl,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    log_started("artists", None, None);

    let mut artists_iterator = client.artists();
    let mut count = 0;

    while let Some(artist) = artists_iterator.next().await? {
        count += 1;

        output_event(&ListEvent::ArtistFound {
            index: count,
            artist,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    log_summary("artists", count, None);

    Ok(())
}

/// Handle the list albums command
pub async fn handle_list_albums(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    log_started("albums", Some(artist), None);

    let mut albums_iterator = client.artist_albums(artist);
    let mut count = 0;

    while let Some(album) = albums_iterator.next().await? {
        count += 1;

        output_event(&ListEvent::AlbumFound {
            index: count,
            album,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    log_summary("albums", count, Some(artist));

    Ok(())
}

/// Handle the list tracks by album command
pub async fn handle_list_tracks_by_album(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    log_started("tracks-by-album", Some(artist), None);

    let mut albums_iterator = client.artist_albums(artist);
    let mut album_count = 0;

    while let Some(album) = albums_iterator.next().await? {
        album_count += 1;

        output_event(&ListEvent::AlbumSection {
            album_index: album_count,
            album: album.clone(),
        });

        // Get tracks for this album
        let mut tracks_iterator = client.album_tracks(&album.name, artist);
        let mut track_idx = 0;

        while let Some(track) = tracks_iterator.next().await.transpose() {
            match track {
                Ok(track) => {
                    track_idx += 1;
                    output_event(&ListEvent::AlbumTrackFound {
                        album_index: album_count,
                        track_index: track_idx,
                        track,
                    });
                }
                Err(e) => {
                    log::error!("Error getting tracks: {e}");
                    break;
                }
            }
        }

        if track_idx == 0 {
            log::warn!("No tracks found in your library for album '{}'", album.name);
        }

        if limit > 0 && album_count >= limit {
            break;
        }
    }

    log_summary("tracks-by-album", album_count, Some(artist));

    Ok(())
}

/// Handle the list tracks command
pub async fn handle_list_tracks(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    log_started("tracks", Some(artist), None);

    let mut tracks_iterator = client.artist_tracks(artist);
    let mut count = 0;

    while let Some(track) = tracks_iterator.next().await? {
        count += 1;

        output_event(&ListEvent::TrackFound {
            index: count,
            track,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    log_summary("tracks", count, Some(artist));

    Ok(())
}

/// Handle the list tracks direct command
pub async fn handle_list_tracks_direct(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    log_started("tracks-direct", Some(artist), None);

    let mut tracks_iterator = client.artist_tracks_direct(artist);
    let mut count = 0;

    while let Some(track) = tracks_iterator.next().await? {
        count += 1;

        output_event(&ListEvent::TrackFound {
            index: count,
            track,
        });

        if limit > 0 && count >= limit {
            break;
        }
    }

    log_summary("tracks-direct", count, Some(artist));

    Ok(())
}

/// Handle the list album tracks command
pub async fn handle_list_album_tracks(
    client: &LastFmEditClientImpl,
    album: &str,
    artist: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log_started("album-tracks", Some(artist), Some(album));

    let mut tracks_iterator = client.album_tracks(album, artist);
    let mut count = 0;

    while let Some(track) = tracks_iterator.next().await? {
        count += 1;

        output_event(&ListEvent::TrackFound {
            index: count,
            track,
        });
    }

    log_summary("album-tracks", count, Some(artist));

    Ok(())
}
