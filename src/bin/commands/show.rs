use super::utils::format_timestamp;
use lastfm_edit::LastFmEditClientImpl;

/// Handle showing details for specific scrobbles by offset
pub async fn handle_show_scrobbles(
    client: &LastFmEditClientImpl,
    offsets: &[u64],
) -> Result<(), Box<dyn std::error::Error>> {
    // No validation needed for 0-based indexing - all u64 values are valid

    let max_offset = *offsets.iter().max().unwrap();

    println!(
        "📋 Showing details for scrobbles at offsets: {}",
        offsets
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Sort offsets for better output organization
    let mut sorted_offsets = offsets.to_vec();
    sorted_offsets.sort_unstable();

    let mut all_scrobbles = Vec::new();

    // Collect scrobbles until we have enough to cover the maximum offset
    let mut page = 1;
    let needed_scrobbles = (max_offset + 1) as usize; // +1 because 0-indexed

    println!("\n📄 Collecting recent scrobbles to reach offset {max_offset}...");

    while all_scrobbles.len() < needed_scrobbles {
        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No more scrobbles found on page {page}");
                    break;
                }

                println!(
                    "  Page {page}: Found {} scrobbles (total: {})",
                    scrobbles.len(),
                    all_scrobbles.len() + scrobbles.len()
                );
                all_scrobbles.extend(scrobbles);
                page += 1;

                // Stop if we've collected enough
                if all_scrobbles.len() >= needed_scrobbles {
                    break;
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    println!("\n📊 Total scrobbles collected: {}", all_scrobbles.len());

    // Check if we have enough scrobbles for all requested offsets
    let unavailable_offsets: Vec<u64> = offsets
        .iter()
        .filter(|&&offset| offset >= all_scrobbles.len() as u64)
        .copied()
        .collect();

    if !unavailable_offsets.is_empty() {
        println!(
            "\n⚠️  The following offsets are not available (you only have {} scrobbles):",
            all_scrobbles.len()
        );
        for offset in &unavailable_offsets {
            println!("    - Offset {offset}");
        }
        println!();
    }

    // Show details for each requested offset
    println!("🎵 Scrobble Details:");
    println!("{}", "=".repeat(80));

    for &offset in &sorted_offsets {
        if offset < all_scrobbles.len() as u64 {
            let scrobble = &all_scrobbles[offset as usize]; // Use offset directly as 0-based index

            println!(
                "\n📍 Offset {offset} ({}{})",
                offset,
                match offset {
                    0 => "st most recent (index 0)",
                    1 => "nd most recent (index 1)",
                    2 => "rd most recent (index 2)",
                    _ => "th most recent",
                }
            );

            println!("   🎤 Artist: {}", scrobble.artist);
            println!("   🎵 Track:  {}", scrobble.name);
            println!("   🔢 Play Count: {}", scrobble.playcount);

            if let Some(album) = &scrobble.album {
                println!("   💿 Album:  {album}");
            } else {
                println!("   💿 Album:  (no album info)");
            }

            if let Some(album_artist) = &scrobble.album_artist {
                if album_artist != &scrobble.artist {
                    println!("   👥 Album Artist: {album_artist}");
                }
            }

            if let Some(timestamp) = scrobble.timestamp {
                println!(
                    "   🕐 Timestamp: {} ({})",
                    timestamp,
                    format_timestamp(timestamp)
                );
            } else {
                println!("   🕐 Timestamp: (no timestamp)");
            }
        }
    }

    if !unavailable_offsets.is_empty() {
        println!(
            "\n❌ Could not show {} offset(s) due to insufficient scrobbles",
            unavailable_offsets.len()
        );
    }

    println!("\n✅ Finished showing scrobble details");

    Ok(())
}
