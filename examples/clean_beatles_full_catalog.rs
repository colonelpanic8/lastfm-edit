#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use regex::Regex;
use std::collections::HashSet;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    println!("=== Beatles Full Catalog Cleanup: Remove All Remastered Suffixes ===\n");

    let artist = "The Beatles";
    let regex = Regex::new(r" - Remastered( \d{4})?$").unwrap();

    println!("🎯 TARGET: Remove all '- Remastered' and '- Remastered YYYY' suffixes from Beatles tracks AND albums");
    println!("🎨 ARTIST: {}", artist);
    println!("🔍 PATTERN: Looking for items ending with '- Remastered' or '- Remastered YYYY'");
    println!("📝 EXAMPLES:");
    println!("   • 'Hey Jude - Remastered 2009' → 'Hey Jude'");
    println!("   • 'Abbey Road - Remastered' → 'Abbey Road'");
    println!("   • 'Sgt. Pepper's Lonely Hearts Club Band - Remastered 2017' → 'Sgt. Pepper's Lonely Hearts Club Band'");
    println!("\n🚀 Starting full catalog scan...\n");

    // Track statistics
    let mut total_tracks_scanned = 0;
    let mut total_albums_scanned = 0;
    let mut remastered_tracks_found = 0;
    let mut remastered_albums_found = 0;
    let mut tracks_successfully_cleaned = 0;
    let mut albums_successfully_cleaned = 0;
    let mut tracks_failed_to_clean = 0;
    let mut albums_failed_to_clean = 0;
    let mut already_cleaned_tracks = HashSet::new();
    let mut already_cleaned_albums = HashSet::new();

    // PART 1: SCAN AND CLEAN TRACKS
    println!("🎵 PART 1: SCANNING AND CLEANING TRACKS");
    println!("{}", "=".repeat(50));

    let mut all_remastered_tracks = Vec::new();

    // Step 1a: Collect all remastered tracks
    println!("🔍 Step 1a: Scanning entire Beatles track catalog for remastered tracks...");
    {
        let mut iterator = client.artist_tracks(artist);
        let mut page_num = 1;

        loop {
            println!("📖 Scanning tracks page {}...", page_num);

            match iterator.next_page().await {
                Ok(Some(page)) => {
                    total_tracks_scanned += page.tracks.len();

                    // Find remastered tracks on this page
                    for track in &page.tracks {
                        if regex.is_match(&track.name) {
                            let base_name = regex.replace(&track.name, "").to_string();
                            if !already_cleaned_tracks.contains(&base_name) {
                                all_remastered_tracks.push(track.clone());
                                already_cleaned_tracks.insert(base_name);
                                remastered_tracks_found += 1;
                            }
                        }
                    }

                    println!(
                        "   📊 Page {}: {} tracks scanned, {} total remastered tracks found so far",
                        page_num,
                        page.tracks.len(),
                        all_remastered_tracks.len()
                    );

                    if !page.has_next_page {
                        println!("📚 Reached end of Beatles track catalog");
                        break;
                    }

                    page_num += 1;
                }
                Ok(None) => {
                    println!("📚 No more track pages available");
                    break;
                }
                Err(e) => {
                    println!("❌ Error fetching track page {}: {}", page_num, e);
                    break;
                }
            }
        }
    }

    // Step 1b: Process all found remastered tracks
    if !all_remastered_tracks.is_empty() {
        println!(
            "\n🎯 Step 1b: Processing {} remastered tracks...",
            all_remastered_tracks.len()
        );
        already_cleaned_tracks.clear();

        for (index, track) in all_remastered_tracks.iter().enumerate() {
            let clean_name = regex.replace(&track.name, "").to_string();

            println!(
                "\n🎵 [{}/{}] Cleaning track: '{}' → '{}'",
                index + 1,
                all_remastered_tracks.len(),
                track.name,
                clean_name
            );

            if already_cleaned_tracks.contains(&clean_name) {
                println!("   ⏭️  Skipping - already processed in this session");
                continue;
            }

            match client.load_edit_form_values(&track.name, artist).await {
                Ok(mut edit_data) => {
                    edit_data.track_name = clean_name.clone();
                    println!("   🔧 Submitting track edit...");

                    match client.edit_scrobble(&edit_data).await {
                        Ok(_response) => {
                            println!("   ✅ Successfully cleaned track: '{}'", clean_name);
                            tracks_successfully_cleaned += 1;
                            already_cleaned_tracks.insert(clean_name);
                        }
                        Err(e) => {
                            println!("   ❌ Error editing track '{}': {}", track.name, e);
                            tracks_failed_to_clean += 1;
                        }
                    }
                }
                Err(e) => {
                    println!(
                        "   ⚠️  Couldn't load edit form for track '{}': {}",
                        track.name, e
                    );
                    tracks_failed_to_clean += 1;
                }
            }

            println!("   ⏳ Waiting 1.2s before next track...");
            tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
        }
    }

    // PART 2: SCAN AND CLEAN ALBUMS
    println!("\n🎼 PART 2: SCANNING AND CLEANING ALBUMS");
    println!("{}", "=".repeat(50));

    let mut all_remastered_albums = Vec::new();

    // Step 2a: Collect all remastered albums
    println!("🔍 Step 2a: Scanning entire Beatles album catalog for remastered albums...");
    {
        let mut iterator = client.artist_albums(artist);
        let mut page_num = 1;

        loop {
            println!("📖 Scanning albums page {}...", page_num);

            match iterator.next_page().await {
                Ok(Some(page)) => {
                    total_albums_scanned += page.albums.len();

                    // Find remastered albums on this page
                    for album in &page.albums {
                        if regex.is_match(&album.name) {
                            let base_name = regex.replace(&album.name, "").to_string();
                            if !already_cleaned_albums.contains(&base_name) {
                                all_remastered_albums.push(album.clone());
                                already_cleaned_albums.insert(base_name);
                                remastered_albums_found += 1;
                            }
                        }
                    }

                    println!(
                        "   📊 Page {}: {} albums scanned, {} total remastered albums found so far",
                        page_num,
                        page.albums.len(),
                        all_remastered_albums.len()
                    );

                    if !page.has_next_page {
                        println!("📚 Reached end of Beatles album catalog");
                        break;
                    }

                    page_num += 1;
                }
                Ok(None) => {
                    println!("📚 No more album pages available");
                    break;
                }
                Err(e) => {
                    println!("❌ Error fetching album page {}: {}", page_num, e);
                    break;
                }
            }
        }
    }

    // Step 2b: Process all found remastered albums
    if !all_remastered_albums.is_empty() {
        println!(
            "\n🎯 Step 2b: Processing {} remastered albums...",
            all_remastered_albums.len()
        );
        already_cleaned_albums.clear();

        for (index, album) in all_remastered_albums.iter().enumerate() {
            let clean_name = regex.replace(&album.name, "").to_string();

            println!(
                "\n💿 [{}/{}] Cleaning album: '{}' → '{}'",
                index + 1,
                all_remastered_albums.len(),
                album.name,
                clean_name
            );

            if already_cleaned_albums.contains(&clean_name) {
                println!("   ⏭️  Skipping - already processed in this session");
                continue;
            }

            match client.edit_album(&album.name, &clean_name, artist).await {
                Ok(_response) => {
                    println!("   ✅ Successfully cleaned album: '{}'", clean_name);
                    albums_successfully_cleaned += 1;
                    already_cleaned_albums.insert(clean_name);
                }
                Err(e) => {
                    println!("   ❌ Error editing album '{}': {}", album.name, e);
                    albums_failed_to_clean += 1;
                }
            }

            println!("   ⏳ Waiting 1.2s before next album...");
            tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
        }
    }

    // Print final statistics
    println!("\n{}", "=".repeat(70));
    println!("🎼 BEATLES FULL CATALOG CLEANUP COMPLETE");
    println!("{}", "=".repeat(70));
    println!("📊 FINAL STATISTICS:");
    println!("   🎵 TRACKS:");
    println!("     • Total tracks scanned: {}", total_tracks_scanned);
    println!(
        "     • Remastered tracks found: {}",
        remastered_tracks_found
    );
    println!(
        "     • Tracks successfully cleaned: {}",
        tracks_successfully_cleaned
    );
    println!("     • Tracks failed to clean: {}", tracks_failed_to_clean);
    println!("   💿 ALBUMS:");
    println!("     • Total albums scanned: {}", total_albums_scanned);
    println!(
        "     • Remastered albums found: {}",
        remastered_albums_found
    );
    println!(
        "     • Albums successfully cleaned: {}",
        albums_successfully_cleaned
    );
    println!("     • Albums failed to clean: {}", albums_failed_to_clean);
    println!("   🎯 TOTALS:");
    println!(
        "     • Total items cleaned: {}",
        tracks_successfully_cleaned + albums_successfully_cleaned
    );
    println!(
        "     • Total items failed: {}",
        tracks_failed_to_clean + albums_failed_to_clean
    );

    if tracks_successfully_cleaned + albums_successfully_cleaned > 0 {
        println!("\n✨ Your Beatles catalog is now cleaner! All 'Remastered' suffixes have been removed from both tracks and albums.");
    }

    if tracks_failed_to_clean + albums_failed_to_clean > 0 {
        println!("\n⚠️  Some items couldn't be cleaned. This usually happens when:");
        println!("   • The item hasn't been scrobbled recently");
        println!("   • The item data isn't in your listening history");
        println!("   • There were temporary server issues");
        println!("\n💡 You can re-run this script later to try cleaning the remaining items.");
    }

    println!("\n🎵 Beatles full catalog cleanup completed!");

    Ok(())
}
