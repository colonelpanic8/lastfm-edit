#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use regex::Regex;
use std::collections::HashSet;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    println!("=== Beatles Album Cleanup: Remove All Remastered Suffixes ===\n");

    let artist = "The Beatles";
    let regex = Regex::new(r" - Remastered( \d{4})?$").unwrap();

    println!(
        "🎯 TARGET: Remove all '- Remastered' and '- Remastered YYYY' suffixes from Beatles albums"
    );
    println!("🎨 ARTIST: {}", artist);
    println!("🔍 PATTERN: Looking for albums ending with '- Remastered' or '- Remastered YYYY'");
    println!("📝 EXAMPLES:");
    println!("   • 'Abbey Road - Remastered 2019' → 'Abbey Road'");
    println!("   • 'Sgt. Pepper's Lonely Hearts Club Band - Remastered' → 'Sgt. Pepper's Lonely Hearts Club Band'");
    println!("   • 'Revolver - Remastered 2009' → 'Revolver'");
    println!("\n🚀 Starting album catalog scan...\n");

    // Album statistics
    let mut total_albums_scanned = 0;
    let mut remastered_albums_found = 0;
    let mut albums_successfully_cleaned = 0;
    let mut albums_failed_to_clean = 0;
    let mut already_cleaned_albums = HashSet::new();

    // Step 1: Collect all remastered albums
    println!("🔍 Step 1: Scanning entire Beatles album catalog for remastered albums...");
    let mut all_remastered_albums = Vec::new();

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

                    // Check if there are more pages
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

    // Step 2: Process all found remastered albums
    if all_remastered_albums.is_empty() {
        println!("\n🎉 No remastered albums found! Your Beatles album catalog is already clean.");
        return Ok(());
    }

    println!(
        "\n🎯 Step 2: Processing {} remastered albums...",
        all_remastered_albums.len()
    );
    already_cleaned_albums.clear(); // Reset for actual processing

    for (index, album) in all_remastered_albums.iter().enumerate() {
        let clean_name = regex.replace(&album.name, "").to_string();

        println!(
            "\n💿 [{}/{}] Cleaning album: '{}' → '{}'",
            index + 1,
            all_remastered_albums.len(),
            album.name,
            clean_name
        );

        // Skip if we've already processed this album name in this session
        if already_cleaned_albums.contains(&clean_name) {
            println!("   ⏭️  Skipping - already processed in this session");
            continue;
        }

        // Edit the album using the new album editing method
        match client.edit_album(&album.name, &clean_name, artist).await {
            Ok(_response) => {
                println!("   ✅ Successfully cleaned album: '{}'", clean_name);
                albums_successfully_cleaned += 1;
                already_cleaned_albums.insert(clean_name);
            }
            Err(e) => {
                println!("   ❌ Error editing album '{}': {}", album.name, e);
                println!("      This might happen if the album hasn't been scrobbled recently");
                albums_failed_to_clean += 1;
            }
        }

        // Add a small delay to be respectful to Last.fm servers
        println!("   ⏳ Waiting 1.2s before next album...");
        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
    }

    // Print final statistics
    println!("\n{}", "=".repeat(60));
    println!("💿 BEATLES ALBUM CLEANUP COMPLETE");
    println!("{}", "=".repeat(60));
    println!("📊 STATISTICS:");
    println!("   • Total albums scanned: {}", total_albums_scanned);
    println!("   • Remastered albums found: {}", remastered_albums_found);
    println!(
        "   • Albums successfully cleaned: {}",
        albums_successfully_cleaned
    );
    println!("   • Albums failed to clean: {}", albums_failed_to_clean);

    if albums_successfully_cleaned > 0 {
        println!("\n✨ Your Beatles album catalog is now cleaner! All 'Remastered' suffixes have been removed from album titles.");
    }

    if albums_failed_to_clean > 0 {
        println!("\n⚠️  Some albums couldn't be cleaned. This usually happens when:");
        println!("   • The album hasn't been scrobbled recently");
        println!("   • The album data isn't in your listening history");
        println!("   • There were temporary server issues");
        println!("\n💡 You can re-run this script later to try cleaning the remaining albums.");
    }

    println!("\n💿 Beatles album cleanup completed!");

    Ok(())
}
