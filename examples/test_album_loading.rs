#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let artist = "The Beatles";

    println!("=== Testing Beatles Album Loading ===\n");

    // Test album iteration
    println!("🔍 Step 1: Testing album iteration...");
    let mut album_iterator = client.artist_albums(artist);

    match album_iterator.next_page().await? {
        Some(page) => {
            println!("✅ Found {} albums on first page", page.albums.len());
            println!("📖 Has next page: {}", page.has_next_page);

            // Display first few albums
            for (i, album) in page.albums.iter().take(5).enumerate() {
                println!("  {}. '{}' - {} plays", i + 1, album.name, album.playcount);
            }

            // Test album metadata loading for the first album with "Remastered" in the name
            if let Some(remastered_album) =
                page.albums.iter().find(|a| a.name.contains("Remastered"))
            {
                println!("\n🎯 Step 2: Testing album metadata loading...");
                println!("📀 Testing album: '{}'", remastered_album.name);

                match client
                    .load_album_edit_form_values(&remastered_album.name, artist)
                    .await
                {
                    Ok(edit_data) => {
                        println!("✅ Successfully loaded album edit form data:");
                        println!(
                            "   📀 Album: '{}' -> '{}'",
                            edit_data.album_name_original, edit_data.album_name
                        );
                        println!("   🎵 Track: '{}'", edit_data.track_name);
                        println!("   🎤 Artist: '{}'", edit_data.artist_name);
                        println!("   ⏰ Timestamp: {}", edit_data.timestamp);

                        // Test what it would look like to clean the album name
                        let regex = regex::Regex::new(r" - Remastered( \d{4})?$").unwrap();
                        if regex.is_match(&edit_data.album_name) {
                            let clean_name = regex.replace(&edit_data.album_name, "").to_string();
                            println!("\n💡 This album could be cleaned:");
                            println!("   From: '{}'", edit_data.album_name);
                            println!("   To:   '{}'", clean_name);
                        }
                    }
                    Err(e) => {
                        println!("❌ Failed to load album edit form: {}", e);
                        println!(
                            "   This might happen if the album hasn't been scrobbled recently"
                        );
                    }
                }
            } else {
                println!("\n⚠️  No albums with 'Remastered' found on first page");
            }
        }
        None => {
            println!("❌ No albums found for {}", artist);
        }
    }

    println!("\n✅ Album loading test completed!");
    Ok(())
}
