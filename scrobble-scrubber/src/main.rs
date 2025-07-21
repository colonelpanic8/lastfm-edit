mod persistence;
mod rewrite;
mod scrubber;

use clap::Parser;
use lastfm_edit::{LastFmClient, LastFmError, Result};
use log::info;
use persistence::FileStorage;
use scrubber::ScrobbleScrubber;
use scrobble_scrubber::Args;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    info!(
        "Starting scrobble-scrubber with interval {}s",
        args.interval
    );

    // Load credentials from environment
    let username = std::env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable required");
    let password = std::env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable required");

    // Create and login to LastFM client
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmClient::new(Box::new(http_client));

    info!("Logging in to Last.fm...");
    client.login(&username, &password).await?;
    info!("Successfully logged in to Last.fm");

    // Create storage
    let storage = FileStorage::new(&args.state_file).map_err(|e| {
        LastFmError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to create storage: {}", e),
        ))
    })?;

    // Create and run scrubber
    let mut scrubber = ScrobbleScrubber::new(args, storage, client).await?;
    scrubber.run().await?;

    Ok(())
}
