use lastfm_edit::{ExactScrobbleEdit, ScrobbleEdit};

fn main() {
    println!("=== Smart ScrobbleEdit Display Examples ===\n");

    // Example 1: Only changing the artist name
    let edit1 = ScrobbleEdit {
        artist_name_original: "The Beatles".to_string(),
        track_name_original: Some("Yesterday".to_string()),
        album_name_original: Some("Help!".to_string()),
        album_artist_name_original: None,

        artist_name: "Beatles, The".to_string(),   // Changed
        track_name: Some("Yesterday".to_string()), // Same
        album_name: Some("Help!".to_string()),     // Same
        album_artist_name: None,                   // Same

        timestamp: None,
        edit_all: false,
    };
    println!("📝 Artist name change:");
    println!("   {edit1}");

    // Example 2: Changing track and album names
    let edit2 = ScrobbleEdit {
        artist_name_original: "Pink Floyd".to_string(),
        track_name_original: Some("Shine on You Crazy Diamond".to_string()),
        album_name_original: Some("Wish You Were Here".to_string()),
        album_artist_name_original: None,

        artist_name: "Pink Floyd".to_string(), // Same
        track_name: Some("Shine On You Crazy Diamond".to_string()), // Changed
        album_name: Some("Wish You Were Here (Remastered)".to_string()), // Changed
        album_artist_name: None,               // Same

        timestamp: Some(1640995200),
        edit_all: true,
    };
    println!("\n📝 Track and album changes:");
    println!("   {edit2}");

    // Example 3: Adding album artist information
    let edit3 = ScrobbleEdit {
        artist_name_original: "Various Artists".to_string(),
        track_name_original: Some("Hotel California".to_string()),
        album_name_original: Some("Greatest Hits Collection".to_string()),
        album_artist_name_original: None,

        artist_name: "Eagles".to_string(),                // Changed
        track_name: Some("Hotel California".to_string()), // Same
        album_name: Some("Hotel California".to_string()), // Changed
        album_artist_name: Some("Eagles".to_string()),    // Added

        timestamp: None,
        edit_all: false,
    };
    println!("\n📝 Multiple changes including adding album artist:");
    println!("   {edit3}");

    // Example 4: No changes (should show "No changes")
    let edit4 = ScrobbleEdit {
        artist_name_original: "Radiohead".to_string(),
        track_name_original: Some("Paranoid Android".to_string()),
        album_name_original: Some("OK Computer".to_string()),
        album_artist_name_original: Some("Radiohead".to_string()),

        artist_name: "Radiohead".to_string(),             // Same
        track_name: Some("Paranoid Android".to_string()), // Same
        album_name: Some("OK Computer".to_string()),      // Same
        album_artist_name: Some("Radiohead".to_string()), // Same

        timestamp: None,
        edit_all: false,
    };
    println!("\n📝 No changes:");
    println!("   {edit4}");

    // Example 5: ExactScrobbleEdit (all fields required)
    let exact_edit = ExactScrobbleEdit {
        artist_name_original: "Led Zeppelin".to_string(),
        track_name_original: "Stairway to Heaven".to_string(),
        album_name_original: "Led Zeppelin IV".to_string(),
        album_artist_name_original: "Led Zeppelin".to_string(),

        artist_name: "Led Zeppelin".to_string(),      // Same
        track_name: "Stairway To Heaven".to_string(), // Changed (capitalization)
        album_name: "Led Zeppelin IV (Remaster)".to_string(), // Changed
        album_artist_name: "Led Zeppelin".to_string(), // Same

        timestamp: 1640995200,
        edit_all: true,
    };
    println!("\n📝 ExactScrobbleEdit changes:");
    println!("   {exact_edit}");

    println!("\n✨ Features demonstrated:");
    println!("   • Only shows fields that are actually changing");
    println!("   • Uses → arrow to show old → new values");
    println!("   • Handles optional fields (None to Some transitions)");
    println!("   • Shows scope with '(all instances)' when edit_all is true");
    println!("   • Shows 'No changes' when nothing is being modified");
}
