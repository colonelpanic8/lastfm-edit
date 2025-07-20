#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{run_track_editor, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // Get artist from command line args or use default
    let args: Vec<String> = std::env::args().collect();
    let artist = if args.len() > 1 {
        args[1].clone()
    } else {
        "The Beatles".to_string()
    };

    println!("Starting Track Editor TUI for artist: {}", artist);
    println!("Setting up Last.fm client...");

    // Setup client
    let client = common::setup_client().await?;
    
    println!("Client ready! Launching TUI...");
    println!("Press any key to continue...");
    
    // Wait for user input before starting TUI
    std::io::stdin().read_line(&mut String::new()).unwrap();

    // Run the TUI
    match run_track_editor(client, artist).await {
        Ok(()) => {
            println!("Track Editor TUI completed successfully!");
        }
        Err(e) => {
            eprintln!("TUI Error: {}", e);
            println!("This might happen if running in a non-interactive terminal.");
            println!("Try running this example in a proper terminal environment.");
        }
    }

    println!("Track Editor TUI has exited. Goodbye!");
    Ok(())
}