#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{LastFmEditClient, Result};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 4 {
        eprintln!("Usage: cargo run --example rename_album -- \"Old Album Name\" \"New Album Name\" \"Artist Name\"");
        eprintln!("Example: cargo run --example rename_album -- \"Abbey Road - Remastered 2019\" \"Abbey Road\" \"The Beatles\"");
        std::process::exit(1);
    }

    let old_album_name = &args[1];
    let new_album_name = &args[2];
    let artist_name = &args[3];

    let client = common::setup_client().await?;

    println!("=== Album Rename Tool ===\n");
    println!("🎨 Artist: {artist_name}");
    println!("💿 Renaming: '{old_album_name}' → '{new_album_name}'");
    println!();

    println!("🔍 Loading album edit form data...");
    match client
        .edit_album(old_album_name, new_album_name, artist_name)
        .await
    {
        Ok(_response) => {
            println!("✅ Successfully renamed album!");
            println!("   From: '{old_album_name}'");
            println!("   To:   '{new_album_name}'");
            println!("   Artist: {artist_name}");
            println!(
                "\n💡 All scrobbles from this album have been updated with the new album name."
            );
        }
        Err(e) => {
            println!("❌ Failed to rename album: {e}");
            println!("\nThis might happen if:");
            println!("   • The album hasn't been scrobbled recently");
            println!("   • The album name doesn't match exactly");
            println!("   • There are temporary server issues");
        }
    }

    Ok(())
}
