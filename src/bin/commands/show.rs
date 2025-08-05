use super::show_output::{HumanReadableShowHandler, JsonShowHandler, ShowEvent, ShowOutputHandler};
use lastfm_edit::LastFmEditClientImpl;

/// Handle showing details for specific scrobbles by offset
pub async fn handle_show_scrobbles(
    client: &LastFmEditClientImpl,
    offsets: &[u64],
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // No validation needed for 0-based indexing - all u64 values are valid

    // Create appropriate handler based on output format
    let mut handler: Box<dyn ShowOutputHandler> = if json_output {
        Box::new(JsonShowHandler::new())
    } else {
        Box::new(HumanReadableShowHandler::new())
    };

    let max_offset = *offsets.iter().max().unwrap();

    // Emit start event
    handler.handle_event(ShowEvent::Started {
        offsets: offsets.to_vec(),
        max_offset,
    });

    // Sort offsets for better output organization
    let mut sorted_offsets = offsets.to_vec();
    sorted_offsets.sort_unstable();

    let mut all_scrobbles = Vec::new();

    // Collect scrobbles until we have enough to cover the maximum offset
    let mut page = 1u32;
    let needed_scrobbles = (max_offset + 1) as usize; // +1 because 0-indexed

    while all_scrobbles.len() < needed_scrobbles {
        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                let scrobbles_found = scrobbles.len();
                if scrobbles_found == 0 {
                    handler.handle_event(ShowEvent::CollectingPage {
                        page,
                        scrobbles_found: 0,
                        total_collected: all_scrobbles.len(),
                    });
                    break;
                }

                all_scrobbles.extend(scrobbles);
                handler.handle_event(ShowEvent::CollectingPage {
                    page,
                    scrobbles_found,
                    total_collected: all_scrobbles.len(),
                });
                page += 1;

                // Stop if we've collected enough
                if all_scrobbles.len() >= needed_scrobbles {
                    break;
                }
            }
            Err(_e) => {
                handler.handle_event(ShowEvent::CollectingPage {
                    page,
                    scrobbles_found: 0,
                    total_collected: all_scrobbles.len(),
                });
                break;
            }
        }
    }

    // Check if we have enough scrobbles for all requested offsets
    let unavailable_offsets: Vec<u64> = offsets
        .iter()
        .filter(|&&offset| offset >= all_scrobbles.len() as u64)
        .copied()
        .collect();

    // Emit collection complete event
    handler.handle_event(ShowEvent::CollectionComplete {
        total_scrobbles: all_scrobbles.len(),
        unavailable_offsets: unavailable_offsets.clone(),
    });

    let mut shown_count = 0;

    // Show details for each requested offset
    for &offset in &sorted_offsets {
        if offset < all_scrobbles.len() as u64 {
            let scrobble = &all_scrobbles[offset as usize];
            handler.handle_event(ShowEvent::ScrobbleDetails {
                offset,
                scrobble: scrobble.clone(),
            });
            shown_count += 1;
        }
    }

    // Emit finished event
    handler.handle_event(ShowEvent::Finished {
        total_shown: shown_count,
        unavailable_count: unavailable_offsets.len(),
    });

    Ok(())
}
