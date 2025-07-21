#[path = "shared/common.rs"]
mod common;

use lastfm_edit::AsyncPaginatedIterator;

use lastfm_edit::Result;
use regex::Regex;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    println!("=== Remaster & Year Removal Tool ===\n");
    println!("🎯 This will remove 'remastered' text and year suffixes from track names");
    println!("📝 Patterns include: '- 2009', '(2009)', '[2009]', '- Remaster', etc.\n");

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "The Beatles".to_string());

    println!("🎵 Processing tracks for artist: {artist}\n");

    // Regex patterns to clean up remaster text and year suffixes
    // Note: Order matters! More specific patterns should come first
    let remaster_patterns = vec![
        // Patterns with "remaster" word (most specific)
        // "Track Name - 2009 Remaster" -> "Track Name"
        Regex::new(r"(?i)\s*-\s*\d{4}\s*remaster(ed)?.*$").unwrap(),
        // "Track Name - Remaster" or "Track Name - Remastered" -> "Track Name"
        Regex::new(r"(?i)\s*-\s*remaster(ed)?.*$").unwrap(),
        // "Track Name (2009 Remaster)" -> "Track Name"
        Regex::new(r"(?i)\s*\(\d{4}\s*remaster(ed)?.*\)\s*$").unwrap(),
        // "Track Name (Remaster)" or "Track Name (Remastered)" -> "Track Name"
        Regex::new(r"(?i)\s*\(remaster(ed)?.*\)\s*$").unwrap(),
        // "Track Name [2009 Remaster]" -> "Track Name"
        Regex::new(r"(?i)\s*\[\d{4}\s*remaster(ed)?.*\]\s*$").unwrap(),
        // "Track Name [Remaster]" or "Track Name [Remastered]" -> "Track Name"
        Regex::new(r"(?i)\s*\[remaster(ed)?.*\]\s*$").unwrap(),
        // "Track Name Remastered" -> "Track Name"
        Regex::new(r"(?i)\s*remaster(ed)?\s*(\d{4})?\s*$").unwrap(),
        // Years that are likely remaster years (1980-2030) - be more conservative
        // "Track Name - 2009" -> "Track Name" (only for likely remaster years)
        Regex::new(r"(?i)\s*-\s*(19[8-9]\d|20[0-3]\d)\s*$").unwrap(),
        // "Track Name (2009)" -> "Track Name" (only for likely remaster years)
        Regex::new(r"(?i)\s*\((19[8-9]\d|20[0-3]\d)\)\s*$").unwrap(),
        // "Track Name [2009]" -> "Track Name" (only for likely remaster years)
        Regex::new(r"(?i)\s*\[(19[8-9]\d|20[0-3]\d)\]\s*$").unwrap(),
        // Other common suffixes that should be removed
        // "Track Name - 2019 Mix" -> "Track Name"
        Regex::new(r"(?i)\s*-\s*\d{4}\s*mix.*$").unwrap(),
        // "Track Name - Mix" -> "Track Name"
        Regex::new(r"(?i)\s*-\s*mix.*$").unwrap(),
    ];

    // First, collect some tracks to process
    let mut tracks_to_process = Vec::new();
    {
        let mut iterator = client.artist_tracks(&artist);
        let mut fetched_count = 0;

        loop {
            match iterator.next().await {
                Ok(Some(track)) => {
                    fetched_count += 1;
                    println!("🔍 [{:3}] Found track: '{}'", fetched_count, track.name);

                    // Check if track name contains remaster text
                    let mut cleaned_name = track.name.clone();
                    let mut needs_cleaning = false;

                    for pattern in &remaster_patterns {
                        if pattern.is_match(&cleaned_name) {
                            cleaned_name = pattern.replace(&cleaned_name, "").trim().to_string();
                            needs_cleaning = true;
                        }
                    }

                    if needs_cleaning && !cleaned_name.is_empty() {
                        tracks_to_process.push((track, cleaned_name));
                    }
                }
                Ok(None) => {
                    println!("\n📚 Fetched all {fetched_count} tracks for {artist}");
                    break;
                }
                Err(e) => {
                    println!("❌ Error fetching tracks: {e}");
                    break;
                }
            }
        }
    }

    println!(
        "\n🧹 Starting remaster removal on {} tracks...\n",
        tracks_to_process.len()
    );

    let mut processed_count = 0;
    let mut edits_made = 0;
    let mut rate_limit_hits = 0;

    // Now process the collected tracks
    for (track, cleaned_name) in tracks_to_process {
        processed_count += 1;
        println!(
            "🔧 [{:3}] Processing: '{}' -> '{}'",
            processed_count, track.name, cleaned_name
        );
        println!("   🔄 Applying change...");

        // Load edit form - this makes an HTTP request
        match client
            .load_edit_form_values(&track.name, &track.artist)
            .await
        {
            Ok(mut edit_data) => {
                // Update track name
                edit_data.track_name = cleaned_name.clone();

                // Submit edit - another HTTP request
                match client.edit_scrobble(&edit_data).await {
                    Ok(_) => {
                        edits_made += 1;
                        println!("   ✅ Successfully cleaned track");
                    }
                    Err(e) => {
                        println!("   ❌ Error editing track: {e}");
                        if e.to_string().contains("RateLimit") {
                            rate_limit_hits += 1;
                            log::info!("Rate limit encountered during edit operation for track '{}' by '{}'", track.name, track.artist);
                            println!("   🚨 RATE LIMIT DETECTED during edit operation!");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                println!("   ⚠️  Couldn't load edit form: {e}");
                if e.to_string().contains("RateLimit") {
                    rate_limit_hits += 1;
                    log::info!(
                        "Rate limit encountered during form load for track '{}' by '{}'",
                        track.name,
                        track.artist
                    );
                    println!("   🚨 RATE LIMIT DETECTED during form load!");
                    break;
                }
            }
        }
    }

    println!("\n=== Summary ===");
    println!("📊 Tracks processed: {processed_count}");
    println!("✏️  Edits made: {edits_made}");
    println!("🚨 Rate limit hits: {rate_limit_hits}");

    if rate_limit_hits > 0 {
        println!("\n🎯 Rate limiting was triggered.");
    } else {
        println!("\n✨ All changes completed successfully!");
    }

    Ok(())
}
