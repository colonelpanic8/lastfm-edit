use super::utils::parse_range;
use crate::LastFmEditClientImpl;

/// Handle deletion of scrobbles from recent pages
pub async fn handle_delete_recent_pages(
    client: &LastFmEditClientImpl,
    pages_range: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_page, end_page) = parse_range(pages_range, "pages")?;

    println!("🗑️  Delete recent scrobbles from pages {start_page}-{end_page}");
    if dry_run {
        println!("🔍 DRY RUN - No actual deletions will be performed");
    }

    let mut total_scrobbles = 0;
    let mut successful_deletions = 0;
    let mut failed_deletions = 0;
    let mut scrobbles_to_delete = Vec::new();

    // Collect scrobbles from the specified pages
    for page in start_page..=end_page {
        println!("\n📄 Processing page {page}...");

        match client.get_recent_scrobbles(page.try_into().unwrap()).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No scrobbles found on page {page}");
                    break; // No more pages
                }

                println!("  Found {} scrobbles on page {page}", scrobbles.len());
                total_scrobbles += scrobbles.len();

                for scrobble in scrobbles {
                    if let Some(timestamp) = scrobble.timestamp {
                        scrobbles_to_delete.push((
                            scrobble.artist.clone(),
                            scrobble.name.clone(),
                            timestamp,
                        ));

                        if dry_run {
                            println!(
                                "    Would delete: '{}' by '{}' ({})",
                                scrobble.name, scrobble.artist, timestamp
                            );
                        }
                    } else {
                        println!(
                            "    ⚠️  Skipping scrobble without timestamp: '{}' by '{}'",
                            scrobble.name, scrobble.artist
                        );
                    }
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    if scrobbles_to_delete.is_empty() {
        println!("\n❌ No scrobbles with timestamps found in the specified page range");
        return Ok(());
    }

    println!("\n📊 Summary:");
    println!("  Total scrobbles found: {total_scrobbles}");
    println!("  Scrobbles with timestamps: {}", scrobbles_to_delete.len());

    if dry_run {
        println!("\n🔍 DRY RUN - No actual deletions performed");
        println!("Use --apply to execute these deletions");
        return Ok(());
    }

    // Actually delete the scrobbles
    println!("\n🗑️  Deleting scrobbles...");

    for (i, (artist, track, timestamp)) in scrobbles_to_delete.iter().enumerate() {
        println!(
            "  {}/{}: Deleting '{}' by '{}'",
            i + 1,
            scrobbles_to_delete.len(),
            track,
            artist
        );

        match client.delete_scrobble(artist, track, *timestamp).await {
            Ok(true) => {
                successful_deletions += 1;
                println!("    ✅ Deleted successfully");
            }
            Ok(false) => {
                failed_deletions += 1;
                println!("    ❌ Deletion failed");
            }
            Err(e) => {
                failed_deletions += 1;
                println!("    ❌ Error: {e}");
            }
        }

        // Add delay between deletions to be respectful to the server
        if i < scrobbles_to_delete.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    println!("\n📊 Final Summary:");
    println!("  Successful deletions: {successful_deletions}");
    println!("  Failed deletions: {failed_deletions}");

    if successful_deletions > 0 {
        println!("\n✅ Deletion session completed!");
    } else if failed_deletions > 0 {
        println!("\n❌ All deletions failed!");
    }

    Ok(())
}

/// Handle deletion of scrobbles from timestamp range
pub async fn handle_delete_timestamp_range(
    client: &LastFmEditClientImpl,
    timestamp_range: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_ts, end_ts) = parse_range(timestamp_range, "timestamp")?;

    println!("🗑️  Delete scrobbles from timestamp range {start_ts}-{end_ts}");
    if dry_run {
        println!("🔍 DRY RUN - No actual deletions will be performed");
    }

    let mut successful_deletions = 0;
    let mut failed_deletions = 0;
    let mut scrobbles_to_delete = Vec::new();

    // Search through recent scrobbles to find ones in the timestamp range
    let max_pages = 20; // Search up to 20 pages of recent scrobbles

    for page in 1..=max_pages {
        println!("📄 Searching page {page} for scrobbles in timestamp range...");

        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No more scrobbles found, stopping search");
                    break;
                }

                let mut found_in_range = 0;
                for scrobble in scrobbles {
                    if let Some(timestamp) = scrobble.timestamp {
                        if timestamp >= start_ts && timestamp <= end_ts {
                            found_in_range += 1;
                            scrobbles_to_delete.push((
                                scrobble.artist.clone(),
                                scrobble.name.clone(),
                                timestamp,
                            ));

                            if dry_run {
                                println!(
                                    "    Would delete: '{}' by '{}' ({})",
                                    scrobble.name, scrobble.artist, timestamp
                                );
                            }
                        }
                    }
                }

                if found_in_range > 0 {
                    println!("  Found {found_in_range} scrobbles in range on page {page}");
                } else {
                    println!("  No scrobbles in range on page {page}");
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    if scrobbles_to_delete.is_empty() {
        println!("\n❌ No scrobbles found in the specified timestamp range");
        return Ok(());
    }

    println!("\n📊 Summary:");
    println!(
        "  Scrobbles in timestamp range: {}",
        scrobbles_to_delete.len()
    );

    if dry_run {
        println!("\n🔍 DRY RUN - No actual deletions performed");
        println!("Use --apply to execute these deletions");
        return Ok(());
    }

    // Actually delete the scrobbles
    println!("\n🗑️  Deleting scrobbles...");

    for (i, (artist, track, timestamp)) in scrobbles_to_delete.iter().enumerate() {
        println!(
            "  {}/{}: Deleting '{}' by '{}'",
            i + 1,
            scrobbles_to_delete.len(),
            track,
            artist
        );

        match client.delete_scrobble(artist, track, *timestamp).await {
            Ok(true) => {
                successful_deletions += 1;
                println!("    ✅ Deleted successfully");
            }
            Ok(false) => {
                failed_deletions += 1;
                println!("    ❌ Deletion failed");
            }
            Err(e) => {
                failed_deletions += 1;
                println!("    ❌ Error: {e}");
            }
        }

        // Add delay between deletions to be respectful to the server
        if i < scrobbles_to_delete.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    println!("\n📊 Final Summary:");
    println!("  Successful deletions: {successful_deletions}");
    println!("  Failed deletions: {failed_deletions}");

    if successful_deletions > 0 {
        println!("\n✅ Deletion session completed!");
    } else if failed_deletions > 0 {
        println!("\n❌ All deletions failed!");
    }

    Ok(())
}

/// Handle deletion of scrobbles by offset from most recent
pub async fn handle_delete_recent_offset(
    client: &LastFmEditClientImpl,
    offset_range: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_offset, end_offset) = parse_range(offset_range, "offset")?;

    // Offsets are already 0-based, so use directly
    let start_index = start_offset;
    let end_index = end_offset;

    println!("🗑️  Delete scrobbles by offset: {start_offset}-{end_offset} ({start_offset}th to {end_offset}th most recent, 0-indexed)");
    if dry_run {
        println!("🔍 DRY RUN - No actual deletions will be performed");
    }

    let mut all_scrobbles = Vec::new();
    let mut successful_deletions = 0;
    let mut failed_deletions = 0;

    // Collect scrobbles until we have enough to cover the offset range
    let mut page = 1;
    let needed_scrobbles = (end_offset + 1) as usize; // +1 because 0-indexed

    println!("\n📄 Collecting recent scrobbles to reach offset {end_offset}...");

    while all_scrobbles.len() < needed_scrobbles {
        match client.get_recent_scrobbles(page.try_into().unwrap()).await {
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

    if all_scrobbles.len() <= start_offset as usize {
        println!("\n❌ Not enough recent scrobbles found. You have {} scrobbles, but requested offset starts at {} (0-indexed)", all_scrobbles.len(), start_offset);
        return Ok(());
    }

    // Extract the scrobbles in the specified offset range
    let actual_end_index = std::cmp::min(end_index as usize, all_scrobbles.len() - 1);
    let scrobbles_in_range = &all_scrobbles[start_index as usize..=actual_end_index];

    println!("\n📊 Summary:");
    println!(
        "  Total recent scrobbles collected: {}",
        all_scrobbles.len()
    );
    println!(
        "  Scrobbles in offset range {}-{}: {}",
        start_offset,
        std::cmp::min(end_offset, (all_scrobbles.len() as u64).saturating_sub(1)),
        scrobbles_in_range.len()
    );

    if dry_run {
        println!("\n🔍 Scrobbles that would be deleted:");
        for (i, scrobble) in scrobbles_in_range.iter().enumerate() {
            let offset_number = start_offset + i as u64;
            if let Some(timestamp) = scrobble.timestamp {
                println!(
                    "    {}: '{}' by '{}' ({})",
                    offset_number, scrobble.name, scrobble.artist, timestamp
                );
            } else {
                println!(
                    "    {}: '{}' by '{}' (no timestamp - cannot delete)",
                    offset_number, scrobble.name, scrobble.artist
                );
            }
        }

        println!("\n🔍 DRY RUN - No actual deletions performed");
        println!("Use --apply to execute these deletions");
        return Ok(());
    }

    // Actually delete the scrobbles
    println!("\n🗑️  Deleting scrobbles by offset...");

    for (i, scrobble) in scrobbles_in_range.iter().enumerate() {
        let offset_number = start_offset + i as u64;

        if let Some(timestamp) = scrobble.timestamp {
            println!(
                "  {}/{}: Deleting offset {} - '{}' by '{}'",
                i + 1,
                scrobbles_in_range.len(),
                offset_number,
                scrobble.name,
                scrobble.artist
            );

            match client
                .delete_scrobble(&scrobble.artist, &scrobble.name, timestamp)
                .await
            {
                Ok(true) => {
                    successful_deletions += 1;
                    println!("    ✅ Deleted successfully");
                }
                Ok(false) => {
                    failed_deletions += 1;
                    println!("    ❌ Deletion failed");
                }
                Err(e) => {
                    failed_deletions += 1;
                    println!("    ❌ Error: {e}");
                }
            }
        } else {
            failed_deletions += 1;
            println!(
                "  {}/{}: Skipping offset {} - '{}' by '{}' (no timestamp)",
                i + 1,
                scrobbles_in_range.len(),
                offset_number,
                scrobble.name,
                scrobble.artist
            );
        }

        // Add delay between deletions to be respectful to the server
        if i < scrobbles_in_range.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    println!("\n📊 Final Summary:");
    println!("  Successful deletions: {successful_deletions}");
    println!("  Failed deletions: {failed_deletions}");

    if successful_deletions > 0 {
        println!("\n✅ Deletion session completed!");
    } else if failed_deletions > 0 {
        println!("\n❌ All deletions failed!");
    }

    Ok(())
}
