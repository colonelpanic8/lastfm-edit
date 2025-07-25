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

    // Create HTTP client and lastfm-edit client
    let http_client = http_client::native::NativeClient::new();
    let client1 = LastFmEditClientImpl::new(Box::new(http_client));

    // Login with first client
    println!("🔐 Logging in with client1...");
    client1.login(&username, &password).await?;
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
                ClientEvent::RateLimited(delay) => {
                    println!("⏳ Client1 monitor: Rate limited for {delay} seconds");
                }
            }
        }
    });

    let monitor2 = tokio::spawn(async move {
        println!("🔍 Client2 monitor started");
        while let Ok(event) = events2.recv().await {
            match event {
                ClientEvent::RateLimited(delay) => {
                    println!("⏳ Client2 monitor: Rate limited for {delay} seconds");
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
        (Some(ClientEvent::RateLimited(delay1)), Some(ClientEvent::RateLimited(delay2))) => {
            println!("🎯 Both clients show rate limiting: {delay1}s and {delay2}s");
            if delay1 == delay2 {
                println!(
                    "✅ SUCCESS: Both clients report the same delay (shared broadcaster working!)"
                );
            } else {
                println!("❌ UNEXPECTED: Different delays reported");
            }
        }
        (None, None) => {
            println!("📊 No rate limiting occurred - this is normal for light usage");
            println!("    In real usage, both clients would see rate limit events when they occur");
        }
        _ => {
            println!("📊 Different event states between clients (unexpected)");
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
