#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use regex::Regex;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    println!("=== Remaster Removal Tool ===\n");
    println!("🎯 This will aggressively remove 'remastered' text from track names");
    println!("📝 Will attempt to trigger rate limiting during edit operations\n");

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "The Beatles".to_string());

    println!("🎵 Processing tracks for artist: {}\n", artist);

    // Regex patterns to clean up remaster text
    let remaster_patterns = vec![
        // "Track Name - 2009 Remaster" -> "Track Name"
        Regex::new(r"(?i)\s*-\s*\d{4}\s*remaster(ed)?.*$").unwrap(),
        // "Track Name - Remaster" or "Track Name - Remastered" -> "Track Name"
        Regex::new(r"(?i)\s*-\s*remaster(ed)?.*$").unwrap(),
        // "Track Name - 2009" (from previous edits) -> "Track Name"
        Regex::new(r"(?i)\s*-\s*\d{4}\s*$").unwrap(),
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
                        if tracks_to_process.len() >= 20 {
                            println!("      📋 Collected {} tracks to clean, stopping fetch", tracks_to_process.len());
                            break;
                        }
                    }

                    if fetched_count >= 100 {
                        println!("      📋 Fetched {} tracks, stopping", fetched_count);
                        break;
                    }
                }
                Ok(None) => {
                    println!("\n📚 Fetched all {} tracks for {}", fetched_count, artist);
                    break;
                }
                Err(e) => {
                    println!("❌ Error fetching tracks: {}", e);
                    break;
                }
            }
        }
    }

    println!("\n🧹 Starting aggressive remaster removal on {} tracks...\n", tracks_to_process.len());

    let mut processed_count = 0;
    let mut edits_made = 0;
    let mut rate_limit_hits = 0;

    // Now process the collected tracks aggressively
    for (track, cleaned_name) in tracks_to_process {
        processed_count += 1;
        println!("🔧 [{:3}] Cleaning: '{}'", processed_count, track.name);
        println!("      🧹 '{}' → '{}'", track.name, cleaned_name);

        // Load edit form - this makes an HTTP request
        match client.load_edit_form_values(&track.name, &track.artist).await {
            Ok(mut edit_data) => {
                // Update track name
                edit_data.track_name = cleaned_name.clone();

                // Submit edit - another HTTP request that might get rate limited
                match client.edit_scrobble(&edit_data).await {
                    Ok(_) => {
                        edits_made += 1;
                        println!("      ✅ Successfully cleaned track");
                    }
                    Err(e) => {
                        println!("      ❌ Error editing track: {}", e);
                        if e.to_string().contains("RateLimit") {
                            rate_limit_hits += 1;
                            println!("      🚨 RATE LIMIT DETECTED during edit operation!");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                println!("      ⚠️  Couldn't load edit form: {}", e);
                if e.to_string().contains("RateLimit") {
                    rate_limit_hits += 1;
                    println!("      🚨 RATE LIMIT DETECTED during form load!");
                    break;
                }
            }
        }

        // NO DELAY - be aggressive to trigger rate limiting
    }

    println!("\n=== Summary ===");
    println!("📊 Tracks processed: {}", processed_count);
    println!("✏️  Edits made: {}", edits_made);
    println!("🚨 Rate limit hits: {}", rate_limit_hits);

    if rate_limit_hits > 0 {
        println!("\n🎯 Rate limiting was triggered.");
    } else {
        println!("\n🤔 No rate limits encountered. Try running again quickly.");
    }

    Ok(())
}