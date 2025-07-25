use lastfm_edit::{ClientEvent, LastFmEditClientImpl};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let username =
        env::var("LASTFM_EDIT_USERNAME").expect("Set LASTFM_EDIT_USERNAME environment variable");
    let password =
        env::var("LASTFM_EDIT_PASSWORD").expect("Set LASTFM_EDIT_PASSWORD environment variable");

    println!("🔧 Demonstrating shared event broadcasting between clients...");

    // Create HTTP client and login to create first client
    let http_client = http_client::native::NativeClient::new();
    println!("🔐 Logging in with client1...");
    let client1 =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), &username, &password)
            .await?;
    println!("✅ Successfully logged in as: {}", client1.username());

    // Create a second client that shares the broadcaster with client1
    let http_client2 = http_client::native::NativeClient::new();
    let client2 = client1.with_shared_broadcaster(Box::new(http_client2));
    println!("🔄 Created client2 with shared broadcaster from client1");

    // Subscribe to events from both clients
    let mut events1 = client1.subscribe();
    let mut events2 = client2.subscribe();
    println!("📡 Subscribed to events from both clients");

    // Spawn background tasks to monitor events from each client
    let monitor1 = tokio::spawn(async move {
        println!("🔍 Client1 monitor started");
        while let Ok(event) = events1.recv().await {
            match event {
                ClientEvent::RequestStarted { request } => {
                    println!(
                        "🚀 Client1 monitor: Started request {}",
                        request.short_description()
                    );
                }
                ClientEvent::RequestCompleted {
                    request,
                    status_code,
                    duration_ms,
                } => {
                    println!(
                        "✅ Client1 monitor: Completed {} - {} ({} ms)",
                        request.short_description(),
                        status_code,
                        duration_ms
                    );
                }
                ClientEvent::RateLimited {
                    delay_seconds,
                    request,
                    rate_limit_type,
                } => {
                    let req_desc = request
                        .as_ref()
                        .map(|r| r.short_description())
                        .unwrap_or_else(|| "unknown request".to_string());
                    println!(
                        "⏳ Client1 monitor: Rate limited ({rate_limit_type:?}) for {delay_seconds} seconds - {req_desc}"
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
                            "✅ Client1 monitor: Edit succeeded '{}' -> '{}' ({duration_ms} ms)",
                            edit.track_name_original, edit.track_name
                        );
                    } else {
                        let error_msg = error_message
                            .as_ref()
                            .map(|s| format!(" - {s}"))
                            .unwrap_or_default();
                        println!(
                            "❌ Client1 monitor: Edit failed '{}' -> '{}' ({duration_ms} ms){error_msg}",
                            edit.track_name_original, edit.track_name
                        );
                    }
                }
            }
        }
    });

    let monitor2 = tokio::spawn(async move {
        println!("🔍 Client2 monitor started");
        while let Ok(event) = events2.recv().await {
            match event {
                ClientEvent::RequestStarted { request } => {
                    println!(
                        "🚀 Client2 monitor: Started request {}",
                        request.short_description()
                    );
                }
                ClientEvent::RequestCompleted {
                    request,
                    status_code,
                    duration_ms,
                } => {
                    println!(
                        "✅ Client2 monitor: Completed {} - {} ({} ms)",
                        request.short_description(),
                        status_code,
                        duration_ms
                    );
                }
                ClientEvent::RateLimited {
                    delay_seconds,
                    request,
                    rate_limit_type,
                } => {
                    let req_desc = request
                        .as_ref()
                        .map(|r| r.short_description())
                        .unwrap_or_else(|| "unknown request".to_string());
                    println!(
                        "⏳ Client2 monitor: Rate limited ({rate_limit_type:?}) for {delay_seconds} seconds - {req_desc}"
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
                            "✅ Client2 monitor: Edit succeeded '{}' -> '{}' ({duration_ms} ms)",
                            edit.track_name_original, edit.track_name
                        );
                    } else {
                        let error_msg = error_message
                            .as_ref()
                            .map(|s| format!(" - {s}"))
                            .unwrap_or_default();
                        println!(
                            "❌ Client2 monitor: Edit failed '{}' -> '{}' ({duration_ms} ms){error_msg}",
                            edit.track_name_original, edit.track_name
                        );
                    }
                }
            }
        }
    });

    // Make a request with client1 that might trigger rate limiting
    println!("📡 Making request with client1...");
    match client1.get_recent_scrobbles(1).await {
        Ok(tracks) => {
            println!("✅ Client1 got {} tracks", tracks.len());
        }
        Err(e) => {
            println!("⚠️ Client1 error: {e}");
        }
    }

    // Check latest event from both clients (should be the same due to shared broadcaster)
    let event1 = client1.latest_event();
    let event2 = client2.latest_event();

    match (event1, event2) {
        (
            Some(ClientEvent::RateLimited {
                delay_seconds: delay1,
                ..
            }),
            Some(ClientEvent::RateLimited {
                delay_seconds: delay2,
                ..
            }),
        ) => {
            println!("🎯 Both clients show rate limiting: {delay1}s and {delay2}s");
            if delay1 == delay2 {
                println!(
                    "✅ SUCCESS: Both clients report the same delay (shared broadcaster working!)"
                );
            } else {
                println!("❌ UNEXPECTED: Different delays reported");
            }
        }
        (
            Some(ClientEvent::RequestCompleted { .. }),
            Some(ClientEvent::RequestCompleted { .. }),
        ) => {
            println!("✅ Both clients show completed requests (shared broadcaster working!)");
        }
        (
            Some(ClientEvent::EditAttempted {
                success: success1, ..
            }),
            Some(ClientEvent::EditAttempted {
                success: success2, ..
            }),
        ) => {
            if success1 == success2 {
                println!("✅ Both clients show same edit result (shared broadcaster working!)");
            } else {
                println!("❌ UNEXPECTED: Different edit results reported");
            }
        }
        (None, None) => {
            println!("📊 No events occurred yet - this is normal");
            println!("    In real usage, both clients would see the same events when they occur");
        }
        _ => {
            println!("📊 Different event states between clients (could be due to timing)");
        }
    }

    // Let monitors run for a bit
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Clean up
    monitor1.abort();
    monitor2.abort();

    println!("🏁 Demo completed!");

    println!("\n📄 Key Points:");
    println!("  • client1.with_shared_broadcaster() creates clients that share event broadcasting");
    println!("  • When any shared client encounters rate limiting, all see the same events");
    println!("  • Use this pattern when you need multiple HTTP clients but want unified rate limit handling");
    Ok(())
}
