use lastfm_edit::{LastFmEditClientImpl, RateLimitEvent};
use tokio::sync::broadcast::error::RecvError;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to see rate limit messages
    env_logger::init();

    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::new(Box::new(http_client));

    // Get a receiver for rate limit events
    let mut rate_limit_receiver = client.rate_limit_events();

    // Spawn a task to handle rate limit events
    let event_handler = tokio::spawn(async move {
        println!("Starting rate limit event monitor...");
        loop {
            match rate_limit_receiver.recv().await {
                Ok(RateLimitEvent::Detected {
                    retry_after,
                    url,
                    status_code,
                    matched_pattern,
                    timestamp,
                }) => {
                    println!("[{}] 🚨 Rate limit detected!", timestamp.format("%H:%M:%S"));
                    println!("  Retry after: {} seconds", retry_after);
                    if let Some(url) = url {
                        println!("  URL: {}", url);
                    }
                    if let Some(status) = status_code {
                        println!("  Status code: {}", status);
                    }
                    if let Some(pattern) = matched_pattern {
                        println!("  Matched pattern: '{}'", pattern);
                    }
                }
                Ok(RateLimitEvent::RetryStarting {
                    delay_seconds,
                    attempt,
                    max_attempts,
                    timestamp,
                }) => {
                    println!(
                        "[{}] 🔄 Starting retry attempt {} of {} (waiting {} seconds)",
                        timestamp.format("%H:%M:%S"),
                        attempt,
                        max_attempts,
                        delay_seconds
                    );
                }
                Ok(RateLimitEvent::RetrySucceeded {
                    attempt,
                    total_delay,
                    timestamp,
                }) => {
                    println!(
                        "[{}] ✅ Retry attempt {} succeeded after {} total seconds",
                        timestamp.format("%H:%M:%S"),
                        attempt,
                        total_delay
                    );
                }
                Ok(RateLimitEvent::RetriesExhausted {
                    final_attempt,
                    total_delay,
                    timestamp,
                }) => {
                    println!(
                        "[{}] ❌ All {} retry attempts failed after {} total seconds",
                        timestamp.format("%H:%M:%S"),
                        final_attempt,
                        total_delay
                    );
                }
                Err(RecvError::Closed) => {
                    println!("Rate limit event channel closed");
                    break;
                }
                Err(RecvError::Lagged(skipped)) => {
                    println!(
                        "⚠️ Rate limit event receiver lagged, {} events skipped",
                        skipped
                    );
                }
            }
        }
    });

    // Attempt to login (this might trigger rate limiting if done too frequently)
    println!("Attempting to login (this might trigger rate limiting)...");

    let username = std::env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable must be set");
    let password = std::env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable must be set");

    match client.login(&username, &password).await {
        Ok(()) => {
            println!("✅ Login successful!");

            // Try to fetch some data which might trigger rate limiting
            println!("Fetching recent tracks (this might trigger rate limiting)...");
            match client.get_recent_scrobbles(1).await {
                Ok(tracks) => {
                    println!("✅ Fetched {} recent tracks", tracks.len());
                }
                Err(e) => {
                    println!("❌ Failed to fetch recent tracks: {}", e);
                }
            }
        }
        Err(e) => {
            println!("❌ Login failed: {}", e);
        }
    }

    // Give some time for any pending rate limit events to be processed
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    println!("Example complete. Press Ctrl+C to exit.");

    // Keep the program running to see rate limit events
    let _ = event_handler.await;

    Ok(())
}
