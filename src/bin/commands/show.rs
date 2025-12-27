use super::show_output::{
    log_collecting_page, log_collection_complete, log_finished, log_started, output_event,
    ShowEvent,
};
use lastfm_edit::LastFmEditClientImpl;

/// Handle showing details for specific scrobbles by offset
pub async fn handle_show_scrobbles(
    client: &LastFmEditClientImpl,
    offsets: &[u64],
) -> Result<(), Box<dyn std::error::Error>> {
    let max_offset = *offsets.iter().max().unwrap();

    log_started(offsets, max_offset);

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
                    log_collecting_page(page, 0, all_scrobbles.len());
                    break;
                }

                all_scrobbles.extend(scrobbles);
                log_collecting_page(page, scrobbles_found, all_scrobbles.len());
                page += 1;

                // Stop if we've collected enough
                if all_scrobbles.len() >= needed_scrobbles {
                    break;
                }
            }
            Err(_e) => {
                log_collecting_page(page, 0, all_scrobbles.len());
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

    log_collection_complete(all_scrobbles.len(), &unavailable_offsets);

    let mut shown_count = 0;

    // Show details for each requested offset
    for &offset in &sorted_offsets {
        if offset < all_scrobbles.len() as u64 {
            let scrobble = &all_scrobbles[offset as usize];
            output_event(&ShowEvent::ScrobbleDetails {
                offset,
                scrobble: scrobble.clone(),
            });
            shown_count += 1;
        } else {
            output_event(&ShowEvent::OffsetUnavailable {
                offset,
                total_available: all_scrobbles.len(),
            });
        }
    }

    log_finished(shown_count, unavailable_offsets.len());

    Ok(())
}
