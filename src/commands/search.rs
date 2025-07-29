use crate::commands::SearchType;
use crate::{LastFmEditClient, LastFmEditClientImpl};

/// Handle the search command for tracks or albums in the user's library
pub async fn handle_search_command(
    client: &LastFmEditClientImpl,
    search_type: SearchType,
    query: &str,
    limit: usize,
    offset: usize,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Assume 50 items per page (common Last.fm page size)
    const ITEMS_PER_PAGE: usize = 50;

    // Calculate starting page and within-page offset
    let starting_page = if offset > 0 {
        (offset / ITEMS_PER_PAGE) + 1
    } else {
        1
    };
    let within_page_offset = offset % ITEMS_PER_PAGE;

    if offset > 0 {
        println!(
            "🔍 Searching for {} containing '{}' (starting from #{})...",
            match search_type {
                SearchType::Tracks => "tracks",
                SearchType::Albums => "albums",
            },
            query,
            offset + 1
        );
    } else {
        println!(
            "🔍 Searching for {} containing '{}'...",
            match search_type {
                SearchType::Tracks => "tracks",
                SearchType::Albums => "albums",
            },
            query
        );
    }

    match search_type {
        SearchType::Tracks => {
            // Create iterator starting from the calculated page
            let mut search_iterator = if starting_page > 1 {
                Box::new(crate::iterator::SearchTracksIterator::with_starting_page(
                    client.clone(),
                    query.to_string(),
                    starting_page as u32,
                ))
            } else {
                client.search_tracks(query)
            };

            let mut total_count = 0;
            let mut displayed_count = 0;
            let should_limit = limit > 0;
            let mut found_any = false;

            // Process results incrementally
            while let Some(track) = search_iterator.next().await? {
                total_count += 1;

                // Skip items until we reach the desired within-page offset
                if total_count <= within_page_offset {
                    continue;
                }

                // Mark that we found at least one result
                if !found_any {
                    found_any = true;
                    println!(); // Add blank line before results
                }

                displayed_count += 1;
                let display_number = offset + displayed_count;

                if verbose {
                    println!(
                        "{}. {} - {} (played {} time{})",
                        display_number,
                        track.artist,
                        track.name,
                        track.playcount,
                        if track.playcount == 1 { "" } else { "s" }
                    );

                    if let Some(album) = &track.album {
                        println!("   Album: {album}");
                    }

                    if let Some(album_artist) = &track.album_artist {
                        if album_artist != &track.artist {
                            println!("   Album Artist: {album_artist}");
                        }
                    }
                    println!(); // Blank line between verbose entries
                } else {
                    println!("{}. {} - {}", display_number, track.artist, track.name);
                }

                if should_limit && displayed_count >= limit {
                    break;
                }
            }

            if !found_any {
                println!("❌ No tracks found matching '{query}'");
            } else {
                println!(
                    "✅ Displayed {} track{}",
                    displayed_count,
                    if displayed_count == 1 { "" } else { "s" }
                );

                if offset > 0 {
                    println!("   (Starting from result #{})", offset + 1);
                }
                if should_limit && displayed_count >= limit {
                    println!("   (Limited to {limit} results)");
                }
            }
        }

        SearchType::Albums => {
            // Create iterator starting from the calculated page
            let mut search_iterator = if starting_page > 1 {
                Box::new(crate::iterator::SearchAlbumsIterator::with_starting_page(
                    client.clone(),
                    query.to_string(),
                    starting_page as u32,
                ))
            } else {
                client.search_albums(query)
            };

            let mut total_count = 0;
            let mut displayed_count = 0;
            let should_limit = limit > 0;
            let mut found_any = false;

            // Process results incrementally
            while let Some(album) = search_iterator.next().await? {
                total_count += 1;

                // Skip items until we reach the desired within-page offset
                if total_count <= within_page_offset {
                    continue;
                }

                // Mark that we found at least one result
                if !found_any {
                    found_any = true;
                    println!(); // Add blank line before results
                }

                displayed_count += 1;
                let display_number = offset + displayed_count;

                if verbose {
                    println!(
                        "{}. {} - {} (played {} time{})",
                        display_number,
                        album.artist,
                        album.name,
                        album.playcount,
                        if album.playcount == 1 { "" } else { "s" }
                    );
                    println!(); // Blank line between verbose entries
                } else {
                    println!("{}. {} - {}", display_number, album.artist, album.name);
                }

                if should_limit && displayed_count >= limit {
                    break;
                }
            }

            if !found_any {
                println!("❌ No albums found matching '{query}'");
            } else {
                println!(
                    "✅ Displayed {} album{}",
                    displayed_count,
                    if displayed_count == 1 { "" } else { "s" }
                );

                if offset > 0 {
                    println!("   (Starting from result #{})", offset + 1);
                }
                if should_limit && displayed_count >= limit {
                    println!("   (Limited to {limit} results)");
                }
            }
        }
    }

    Ok(())
}
