use super::search_output::{log_no_results, log_started, log_summary, output_event, SearchEvent};
use super::SearchType;
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};

/// Number of items per page in Last.fm search results
const ITEMS_PER_PAGE: usize = 30;

/// Handle the search command for tracks or albums in the user's library
pub async fn handle_search_command(
    client: &LastFmEditClientImpl,
    search_type: SearchType,
    query: &str,
    limit: usize,
    offset: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let search_type_str = match search_type {
        SearchType::Tracks => "tracks",
        SearchType::Albums => "albums",
        SearchType::Artists => "artists",
    };

    log_started(search_type_str, query, offset);

    // Calculate starting page and within-page offset
    let starting_page = if offset > 0 {
        (offset / ITEMS_PER_PAGE) + 1
    } else {
        1
    };
    let within_page_offset = offset % ITEMS_PER_PAGE;

    match search_type {
        SearchType::Tracks => {
            // Create iterator starting from the calculated page
            let mut search_iterator = if starting_page > 1 {
                Box::new(lastfm_edit::SearchTracksIterator::with_starting_page(
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

            // Process results incrementally
            while let Some(track) = search_iterator.next().await? {
                total_count += 1;

                // Skip items until we reach the desired within-page offset
                if total_count <= within_page_offset {
                    continue;
                }

                displayed_count += 1;
                let display_number = offset + displayed_count;

                output_event(&SearchEvent::TrackFound {
                    index: display_number,
                    track,
                });

                if should_limit && displayed_count >= limit {
                    break;
                }
            }

            if displayed_count == 0 {
                log_no_results(query);
            } else {
                log_summary(displayed_count, offset, limit);
            }
        }

        SearchType::Albums => {
            // Create iterator starting from the calculated page
            let mut search_iterator = if starting_page > 1 {
                Box::new(lastfm_edit::SearchAlbumsIterator::with_starting_page(
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

            // Process results incrementally
            while let Some(album) = search_iterator.next().await? {
                total_count += 1;

                // Skip items until we reach the desired within-page offset
                if total_count <= within_page_offset {
                    continue;
                }

                displayed_count += 1;
                let display_number = offset + displayed_count;

                output_event(&SearchEvent::AlbumFound {
                    index: display_number,
                    album,
                });

                if should_limit && displayed_count >= limit {
                    break;
                }
            }

            if displayed_count == 0 {
                log_no_results(query);
            } else {
                log_summary(displayed_count, offset, limit);
            }
        }

        SearchType::Artists => {
            // Create iterator starting from the calculated page
            let mut search_iterator = if starting_page > 1 {
                Box::new(lastfm_edit::SearchArtistsIterator::with_starting_page(
                    client.clone(),
                    query.to_string(),
                    starting_page as u32,
                ))
            } else {
                client.search_artists(query)
            };

            let mut total_count = 0;
            let mut displayed_count = 0;
            let should_limit = limit > 0;

            // Process results incrementally
            while let Some(artist) = search_iterator.next().await? {
                total_count += 1;

                // Skip items until we reach the desired within-page offset
                if total_count <= within_page_offset {
                    continue;
                }

                displayed_count += 1;
                let display_number = offset + displayed_count;

                output_event(&SearchEvent::ArtistFound {
                    index: display_number,
                    artist,
                });

                if should_limit && displayed_count >= limit {
                    break;
                }
            }

            if displayed_count == 0 {
                log_no_results(query);
            } else {
                log_summary(displayed_count, offset, limit);
            }
        }
    }

    Ok(())
}
