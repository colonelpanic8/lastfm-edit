use lastfm_edit::{ClientEvent, LastFmEditClientImpl};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let username =
        env::var("LASTFM_EDIT_USERNAME").expect("Set LASTFM_EDIT_USERNAME environment variable");
    let password =
        env::var("LASTFM_EDIT_PASSWORD").expect("Set LASTFM_EDIT_PASSWORD environment variable");

    // Login and create client
    let http_client = http_client::native::NativeClient::new();
    println!("Logging in as {username}...");
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), &username, &password)
            .await?;

    // Subscribe to client events before any operations
    let mut events = client.subscribe();

    // Spawn a background task to monitor events
    let event_monitor = tokio::spawn(async move {
        println!("🔍 Monitoring client events...");
        while let Ok(event) = events.recv().await {
            match event {
                ClientEvent::RequestStarted { request } => {
                    println!("🚀 Starting request: {}", request.short_description());
                }
                ClientEvent::RequestCompleted {
                    request,
                    status_code,
                    duration_ms,
                } => {
                    println!(
                        "✅ Completed request: {} - {} ({} ms)",
                        request.short_description(),
                        status_code,
                        duration_ms
                    );
                }
                ClientEvent::RateLimited {
                    delay_seconds,
                    request,
                    rate_limit_type,
                    rate_limit_timestamp,
                } => {
                    let req_desc = request
                        .as_ref()
                        .map(|r| r.short_description())
                        .unwrap_or_else(|| "unknown request".to_string());
                    println!(
                        "⏳ Rate limited ({rate_limit_type:?})! {req_desc} - Waiting {delay_seconds} seconds (at timestamp {rate_limit_timestamp})"
                    );
                }
                ClientEvent::RateLimitEnded {
                    request,
                    rate_limit_type,
                    total_rate_limit_duration_seconds,
                } => {
                    println!(
                        "🎉 Rate limiting ended ({rate_limit_type:?}) after {total_rate_limit_duration_seconds} seconds - {}",
                        request.short_description()
                    );
                }
                ClientEvent::EditAttempted {
                    edit,
                    success,
                    error_message,
                    duration_ms,
                } => {
                    if success {
                        println!(
                            "✅ Edit succeeded: '{}' -> '{}' ({duration_ms} ms)",
                            edit.track_name_original, edit.track_name
                        );
                    } else {
                        let error_msg = error_message
                            .as_ref()
                            .map(|s| format!(" - {s}"))
                            .unwrap_or_default();
                        println!(
                            "❌ Edit failed: '{}' -> '{}' ({duration_ms} ms){error_msg}",
                            edit.track_name_original, edit.track_name
                        );
                    }
                }
            }
        }
    });

    println!("✅ Successfully logged in as: {}", client.username());

    // Check latest event after login
    if let Some(event) = client.latest_event() {
        match event {
            ClientEvent::RequestStarted { request } => {
                println!(
                    "📊 Latest event: Started request {}",
                    request.short_description()
                );
            }
            ClientEvent::RequestCompleted {
                request,
                status_code,
                duration_ms,
            } => {
                println!(
                    "📊 Latest event: Completed request {} - {} ({} ms)",
                    request.short_description(),
                    status_code,
                    duration_ms
                );
            }
            ClientEvent::RateLimited {
                delay_seconds,
                request,
                rate_limit_type,
                rate_limit_timestamp,
            } => {
                let req_desc = request
                    .as_ref()
                    .map(|r| r.short_description())
                    .unwrap_or_else(|| "unknown request".to_string());
                println!(
                    "📊 Latest event: Rate limited ({rate_limit_type:?}) for {delay_seconds} seconds - {req_desc} (at timestamp {rate_limit_timestamp})"
                );
            }
            ClientEvent::RateLimitEnded {
                request,
                rate_limit_type,
                total_rate_limit_duration_seconds,
            } => {
                println!(
                    "📊 Latest event: Rate limiting ended ({rate_limit_type:?}) after {total_rate_limit_duration_seconds} seconds - {}",
                    request.short_description()
                );
            }
            ClientEvent::EditAttempted {
                edit,
                success,
                error_message,
                duration_ms,
            } => {
                if success {
                    println!(
                        "📊 Latest event: Edit succeeded '{}' -> '{}' ({duration_ms} ms)",
                        edit.track_name_original, edit.track_name
                    );
                } else {
                    let error_msg = error_message
                        .as_ref()
                        .map(|s| format!(" - {s}"))
                        .unwrap_or_default();
                    println!(
                        "📊 Latest event: Edit failed '{}' -> '{}' ({duration_ms} ms){error_msg}",
                        edit.track_name_original, edit.track_name
                    );
                }
            }
        }
    } else {
        println!("📊 No events have occurred yet");
    }

    // Make some requests that might trigger rate limiting
    println!("🎵 Fetching recent tracks to potentially trigger rate limiting...");

    for page in 1..=3 {
        println!("📄 Fetching page {page}...");
        match client.get_recent_scrobbles(page).await {
            Ok(tracks) => {
                println!("✅ Got {} tracks from page {page}", tracks.len());
            }
            Err(e) => {
                println!("❌ Error on page {page}: {e}");
            }
        }

        // Check if we're currently rate limited
        if let Some(ClientEvent::RateLimited { delay_seconds, .. }) = client.latest_event() {
            println!(
                "🛑 Currently rate limited for {delay_seconds} seconds according to latest event"
            );
        }

        // Small delay between requests
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    println!("🏁 Done! Event monitor will continue running...");

    // Let the event monitor run for a bit longer to catch any final events
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Cancel the event monitor
    event_monitor.abort();

    Ok(())
}
