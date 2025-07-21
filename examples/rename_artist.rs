#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <old_artist_name> <new_artist_name>", args[0]);
        eprintln!("Example: {} \"The Beatles\" \"Beatles\"", args[0]);
        std::process::exit(1);
    }

    let old_artist_name = &args[1];
    let new_artist_name = &args[2];

    println!("=== Artist Rename Tool ===\n");
    println!("🎯 This will rename all tracks from one artist to another");
    println!("📝 Old artist: '{old_artist_name}'");
    println!("📝 New artist: '{new_artist_name}'");
    println!("⚠️  This will edit ALL tracks that are found in your recent scrobbles!\n");

    print!("Are you sure you want to continue? [y/N]: ");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    let response = input.trim().to_lowercase();

    if response != "y" && response != "yes" {
        println!("Operation cancelled.");
        return Ok(());
    }

    println!("\n🔍 Starting artist rename operation...\n");

    match client.edit_artist(old_artist_name, new_artist_name).await {
        Ok(response) => {
            if response.success {
                println!("✅ Success!");
                if let Some(message) = response.message {
                    println!("📋 {message}");
                }
            } else {
                println!("❌ Operation failed");
                if let Some(message) = response.message {
                    println!("📋 {message}");
                }
            }
        }
        Err(e) => {
            println!("❌ Error during artist rename: {e}");
        }
    }

    Ok(())
}
