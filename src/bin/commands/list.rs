use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};

/// Handle the list albums command
pub async fn handle_list_albums(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
    verbose: bool,
    format: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("🎵 Listing albums for artist: '{artist}'");

    let mut albums_iterator = client.artist_albums(artist);
    let mut count = 0;

    while let Some(album) = albums_iterator.next().await? {
        count += 1;

        if format {
            if verbose {
                println!("  [{count:3}] {album} ({} plays)", album.playcount);
            } else {
                println!("  [{count:3}] {album}");
            }
        } else if verbose {
            println!("  [{count:3}] {} ({} plays)", album.name, album.playcount);
        } else {
            println!("  [{count:3}] {}", album.name);
        }

        if limit > 0 && count >= limit {
            break;
        }
    }

    if count == 0 {
        println!("  No albums found for this artist.");
    } else {
        println!(
            "\nFound {} album{} for '{artist}'",
            count,
            if count == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Handle the list tracks by album command
pub async fn handle_list_tracks_by_album(
    client: &LastFmEditClientImpl,
    artist: &str,
    limit: usize,
    verbose: bool,
    format: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("🎵 Listing tracks by album for artist: '{artist}'");

    let mut albums_iterator = client.artist_albums(artist);
    let mut album_count = 0;
    let mut total_track_count = 0;

    while let Some(album) = albums_iterator.next().await? {
        album_count += 1;

        if verbose {
            println!(
                "\n📀 Album {}: {} ({} plays)",
                album_count, album.name, album.playcount
            );
        } else {
            println!("\n📀 Album {}: {}", album_count, album.name);
        }

        // Get tracks for this album
        match client.get_album_tracks(&album.name, artist).await {
            Ok(tracks) => {
                if tracks.is_empty() {
                    println!("    (No tracks found in your library for this album)");
                } else {
                    for (track_idx, track) in tracks.iter().enumerate() {
                        total_track_count += 1;
                        if format {
                            if verbose {
                                println!(
                                    "    [{:2}] {track} ({} plays)",
                                    track_idx + 1,
                                    track.playcount
                                );
                            } else {
                                println!("    [{:2}] {track}", track_idx + 1);
                            }
                        } else if verbose {
                            println!(
                                "    [{:2}] {} ({} plays)",
                                track_idx + 1,
                                track.name,
                                track.playcount
                            );
                        } else {
                            println!("    [{:2}] {}", track_idx + 1, track.name);
                        }
                    }
                }
            }
            Err(e) => {
                println!("    ❌ Error getting tracks: {e}");
            }
        }

        if limit > 0 && album_count >= limit {
            break;
        }
    }

    if album_count == 0 {
        println!("  No albums found for this artist.");
    } else {
        println!(
            "\nListed {} album{} with {} total tracks for '{artist}'",
            album_count,
            if album_count == 1 { "" } else { "s" },
            total_track_count
        );
    }

    Ok(())
}
