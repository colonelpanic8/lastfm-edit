use lastfm_edit::{Html, LastFmEditClient};
use std::fs;

#[test]
fn test_recent_tracks_parsing() {
    // Read the saved HTML from test fixtures
    let html_content = fs::read_to_string("tests/fixtures/recent_tracks_page_1.html")
        .expect("Could not read test fixture file");

    let document = Html::parse_document(&html_content);

    // Create a mock client for testing parsing
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClient::new(Box::new(http_client));

    // Test the parsing
    let tracks = client.parse_recent_scrobbles(&document).unwrap();

    println!("Parsed {} tracks from recent scrobbles page", tracks.len());

    // We should get 50 tracks per page
    assert_eq!(
        tracks.len(),
        50,
        "Should parse 50 tracks from recent scrobbles page"
    );

    // Print first few tracks for debugging
    for (i, track) in tracks.iter().take(5).enumerate() {
        println!(
            "{}. {} - {} (album: {:?}, timestamp: {:?})",
            i + 1,
            track.artist,
            track.name,
            track.album,
            track.timestamp
        );
    }

    // Test that at least some tracks have album information
    let tracks_with_albums = tracks.iter().filter(|t| t.album.is_some()).count();
    println!("Found {tracks_with_albums} tracks with album information");

    // We should have at least some tracks with album info (based on the test fixture)
    assert!(
        tracks_with_albums > 0,
        "Expected to find at least some tracks with album information"
    );
}

#[test]
fn test_debug_html_structure() {
    // Read the saved HTML from test fixtures
    let html_content = fs::read_to_string("tests/fixtures/recent_tracks_page_1.html")
        .expect("Could not read test fixture file");

    let document = Html::parse_document(&html_content);

    // Check for chartlist tables
    let table_selector = scraper::Selector::parse("table.chartlist").unwrap();
    let tables: Vec<_> = document.select(&table_selector).collect();
    println!("Found {} chartlist tables", tables.len());

    if let Some(table) = tables.first() {
        let row_selector = scraper::Selector::parse("tbody tr").unwrap();
        let rows: Vec<_> = table.select(&row_selector).collect();
        println!("Found {} rows in first chartlist table", rows.len());

        // Check selectors we're using
        let name_selector = scraper::Selector::parse(".chartlist-name a").unwrap();
        let artist_selector = scraper::Selector::parse(".chartlist-artist a").unwrap();

        let mut successful_parses = 0;
        for (i, row) in rows.iter().take(10).enumerate() {
            let name = row
                .select(&name_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string());
            let artist = row
                .select(&artist_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string());

            if name.is_some() && artist.is_some() {
                successful_parses += 1;
                println!(
                    "Row {}: {} - {}",
                    i + 1,
                    artist.unwrap_or_default(),
                    name.unwrap_or_default()
                );
            } else {
                println!(
                    "Row {}: Failed to parse (name: {:?}, artist: {:?})",
                    i + 1,
                    name,
                    artist
                );
            }
        }

        println!(
            "Successfully parsed {}/{} rows examined",
            successful_parses,
            rows.len().min(10)
        );
    }
}
