#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    println!("=== Examining Beatles Albums for Intelligent Renaming ===\n");

    let artist = "The Beatles";
    let mut all_albums = Vec::new();

    // Collect all Beatles albums
    println!("🔍 Scanning Beatles album catalog...");
    let mut iterator = client.artist_albums(artist);
    let mut page_num = 1;

    loop {
        println!("📖 Scanning page {}...", page_num);

        match iterator.next_page().await {
            Ok(Some(page)) => {
                println!("   Found {} albums on page {}", page.albums.len(), page_num);
                all_albums.extend(page.albums);

                if !page.has_next_page {
                    break;
                }
                page_num += 1;
            }
            Ok(None) => break,
            Err(e) => {
                println!("❌ Error: {}", e);
                break;
            }
        }
    }

    println!("\n📊 Found {} total Beatles albums", all_albums.len());
    println!("\n📝 Complete Beatles Album List:");
    println!("{}", "=".repeat(80));

    for (i, album) in all_albums.iter().enumerate() {
        println!("{:3}. '{}' ({} plays)", i + 1, album.name, album.playcount);
    }

    println!("\n🎯 Analysis for Intelligent Renaming:");
    println!("{}", "=".repeat(80));

    // Look for patterns that need cleaning
    let mut remastered_albums = Vec::new();
    let mut deluxe_albums = Vec::new();
    let mut special_editions = Vec::new();
    let mut other_variants = Vec::new();

    for album in &all_albums {
        let name = &album.name;

        if name.contains("Remastered") {
            remastered_albums.push(album);
        } else if name.contains("Deluxe") || name.contains("Super Deluxe") {
            deluxe_albums.push(album);
        } else if name.contains("Special Edition") || name.contains("Anniversary") {
            special_editions.push(album);
        } else if name.contains("(")
            || name.contains("[")
            || name.contains("Mono")
            || name.contains("Stereo")
        {
            other_variants.push(album);
        }
    }

    if !remastered_albums.is_empty() {
        println!("\n🔧 REMASTERED ALBUMS ({}):", remastered_albums.len());
        for album in &remastered_albums {
            println!("   • '{}'", album.name);
        }
    }

    if !deluxe_albums.is_empty() {
        println!(
            "\n💎 DELUXE/SUPER DELUXE EDITIONS ({}):",
            deluxe_albums.len()
        );
        for album in &deluxe_albums {
            println!("   • '{}'", album.name);
        }
    }

    if !special_editions.is_empty() {
        println!("\n🎁 SPECIAL EDITIONS ({}):", special_editions.len());
        for album in &special_editions {
            println!("   • '{}'", album.name);
        }
    }

    if !other_variants.is_empty() {
        println!("\n🔀 OTHER VARIANTS ({}):", other_variants.len());
        for album in &other_variants {
            println!("   • '{}'", album.name);
        }
    }

    println!("\n💡 RECOMMENDED INTELLIGENT RENAMES:");
    println!("{}", "=".repeat(80));

    // Now I'll examine the actual data and make intelligent rename suggestions
    // This will be printed so the user can see my analysis

    Ok(())
}
