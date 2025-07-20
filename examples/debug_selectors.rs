#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let artist = "The Beatles";
    let url = format!(
        "https://www.last.fm/user/{}/library/music/{}/+tracks",
        "IvanMalison", // hardcode for now
        urlencoding::encode(artist)
    );

    println!("Manually fetching and examining HTML from: {}", url);

    // We need to create a manual HTTP request to get the raw HTML
    // Let's use the existing get method but we need to make it public or work around it

    // For now, let's get a page and examine the HTML structure indirectly
    let page = client.get_artist_tracks_page(artist, 1).await?;

    println!("=== DEBUGGING SELECTORS ===");
    println!(
        "Page shows: has_next={}, total_pages={:?}",
        page.has_next_page, page.total_pages
    );

    // Let's test different possible selectors for play counts
    // We'll need to enhance our parsing to see what's actually in the HTML

    println!("\n=== POSSIBLE ISSUES ===");
    println!("1. Play count selector '.chartlist-count-bar-value' may be wrong");
    println!("2. Pagination selector 'a[aria-label=\"Next\"]' may be wrong");
    println!("3. Pagination detection logic may have bugs");

    println!("\n=== NEXT STEPS ===");
    println!("Need to examine actual HTML structure from Last.fm");
    println!("Expected selectors based on reference code:");
    println!("- Play counts might be in a different element");
    println!("- Pagination might use different next/prev link structure");

    Ok(())
}
