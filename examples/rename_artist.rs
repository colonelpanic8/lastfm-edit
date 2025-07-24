#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage:");
        eprintln!("  {} all <old_artist> <new_artist>", args[0]);
        eprintln!("  {} track <track_name> <old_artist> <new_artist>", args[0]);
        eprintln!("  {} album <album_name> <old_artist> <new_artist>", args[0]);
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  {} all \"The Beatles\" \"Beatles\"", args[0]);
        eprintln!(
            "  {} track \"Hey Jude\" \"The Beatles\" \"Beatles\"",
            args[0]
        );
        eprintln!(
            "  {} album \"Abbey Road\" \"The Beatles\" \"Beatles\"",
            args[0]
        );
        std::process::exit(1);
    }

    let mode = &args[1];

    match mode.as_str() {
        "all" => {
            if args.len() != 4 {
                eprintln!("Usage: {} all <old_artist> <new_artist>", args[0]);
                std::process::exit(1);
            }
            let old_artist = &args[2];
            let new_artist = &args[3];
            rename_all_tracks(&mut client, old_artist, new_artist).await
        }
        "track" => {
            if args.len() != 5 {
                eprintln!(
                    "Usage: {} track <track_name> <old_artist> <new_artist>",
                    args[0]
                );
                std::process::exit(1);
            }
            let track_name = &args[2];
            let old_artist = &args[3];
            let new_artist = &args[4];
            rename_single_track(&mut client, track_name, old_artist, new_artist).await
        }
        "album" => {
            if args.len() != 5 {
                eprintln!(
                    "Usage: {} album <album_name> <old_artist> <new_artist>",
                    args[0]
                );
                std::process::exit(1);
            }
            let album_name = &args[2];
            let old_artist = &args[3];
            let new_artist = &args[4];
            rename_album_tracks(&mut client, album_name, old_artist, new_artist).await
        }
        _ => {
            eprintln!("Invalid mode '{mode}'. Use 'all', 'track', or 'album'");
            std::process::exit(1);
        }
    }
}

async fn rename_all_tracks(
    client: &mut dyn lastfm_edit::LastFmEditClient,
    old_artist: &str,
    new_artist: &str,
) -> Result<()> {
    println!("=== Artist Rename Tool - All Tracks ===\n");
    println!("🎯 This will rename ALL tracks from one artist to another");
    println!("📝 Old artist: '{old_artist}'");
    println!("📝 New artist: '{new_artist}'");
    println!("⚠️  This will edit ALL tracks that are found in your recent scrobbles!\n");

    if !confirm_operation()? {
        return Ok(());
    }

    println!("\n🔍 Starting artist rename operation...\n");

    match client.edit_artist(old_artist, new_artist).await {
        Ok(response) => print_result(response),
        Err(e) => println!("❌ Error during artist rename: {e}"),
    }

    Ok(())
}

async fn rename_single_track(
    client: &mut dyn lastfm_edit::LastFmEditClient,
    track_name: &str,
    old_artist: &str,
    new_artist: &str,
) -> Result<()> {
    println!("=== Artist Rename Tool - Single Track ===\n");
    println!("🎯 This will rename the artist for a specific track");
    println!("🎵 Track: '{track_name}'");
    println!("📝 Old artist: '{old_artist}'");
    println!("📝 New artist: '{new_artist}'");
    println!("⚠️  This will only edit this specific track if found in recent scrobbles!\n");

    if !confirm_operation()? {
        return Ok(());
    }

    println!("\n🔍 Starting track artist rename...\n");

    match client
        .edit_artist_for_track(track_name, old_artist, new_artist)
        .await
    {
        Ok(response) => print_result(response),
        Err(e) => println!("❌ Error during track artist rename: {e}"),
    }

    Ok(())
}

async fn rename_album_tracks(
    client: &mut dyn lastfm_edit::LastFmEditClient,
    album_name: &str,
    old_artist: &str,
    new_artist: &str,
) -> Result<()> {
    println!("=== Artist Rename Tool - Album Tracks ===\n");
    println!("🎯 This will rename the artist for all tracks in a specific album");
    println!("💿 Album: '{album_name}'");
    println!("📝 Old artist: '{old_artist}'");
    println!("📝 New artist: '{new_artist}'");
    println!("⚠️  This will edit all tracks in this album that are found in recent scrobbles!\n");

    if !confirm_operation()? {
        return Ok(());
    }

    println!("\n🔍 Starting album artist rename...\n");

    match client
        .edit_artist_for_album(album_name, old_artist, new_artist)
        .await
    {
        Ok(response) => print_result(response),
        Err(e) => println!("❌ Error during album artist rename: {e}"),
    }

    Ok(())
}

fn confirm_operation() -> Result<bool> {
    print!("Are you sure you want to continue? [y/N]: ");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    let response = input.trim().to_lowercase();

    if response != "y" && response != "yes" {
        println!("Operation cancelled.");
        return Ok(false);
    }

    Ok(true)
}

fn print_result(response: lastfm_edit::EditResponse) {
    if response.success() {
        println!("✅ Success!");
        if let Some(message) = response.message() {
            println!("📋 {message}");
        }
    } else {
        println!("❌ Operation failed");
        if let Some(message) = response.message() {
            println!("📋 {message}");
        }
    }
}
