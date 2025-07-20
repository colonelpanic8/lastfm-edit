mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let artist = "The Beatles";

    println!("=== Testing specific pages ===");

    // Test page 1
    println!("\n--- Page 1 ---");
    let page1 = client.get_artist_tracks_page(artist, 1).await?;
    println!(
        "Page 1: {} tracks, has_next: {}, total_pages: {:?}",
        page1.tracks.len(),
        page1.has_next_page,
        page1.total_pages
    );

    // Test page 2 directly
    println!("\n--- Page 2 ---");
    let expected_url =
        "https://www.last.fm/user/IvanMalison/library/music/The+Beatles/+tracks?page=2";
    println!("Expected URL: {}", expected_url);
    let page2 = client.get_artist_tracks_page(artist, 2).await?;
    println!(
        "Page 2: {} tracks, has_next: {}, total_pages: {:?}",
        page2.tracks.len(),
        page2.has_next_page,
        page2.total_pages
    );

    if page2.tracks.is_empty() {
        println!("⚠️  Page 2 returned 0 tracks - this suggests parsing issue");
    } else {
        println!("Sample tracks from page 2:");
        for (i, track) in page2.tracks.iter().take(3).enumerate() {
            println!("  {}. {} - {} plays", i + 1, track.name, track.playcount);
        }
    }

    // Test page 3 directly
    println!("\n--- Page 3 ---");
    let page3 = client.get_artist_tracks_page(artist, 3).await?;
    println!(
        "Page 3: {} tracks, has_next: {}, total_pages: {:?}",
        page3.tracks.len(),
        page3.has_next_page,
        page3.total_pages
    );

    Ok(())
}
