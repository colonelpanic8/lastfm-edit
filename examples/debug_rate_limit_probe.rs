//! Read-only probe: GET the edit-form URL the executor keeps failing on and
//! report the status plus any rate-limit pattern matches in the body.

use lastfm_edit::{LastFmEditClientImpl, SessionPersistence};

const PATTERNS: &[&str] = &[
    "you've tried to log in too many times",
    "you're requesting too many pages",
    "slow down",
    "too fast",
    "rate limit",
    "throttled",
    "temporarily blocked",
    "temporarily restricted",
    "captcha",
    "verify you're human",
    "prove you're not a robot",
    "security check",
    "service temporarily unavailable",
    "quota exceeded",
    "limit exceeded",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let username = std::env::var("LASTFM_EDIT_USERNAME").unwrap_or_else(|_| "IvanMalison".into());
    let session = SessionPersistence::load_session(&username)?;
    let base_url = session.base_url.clone();
    let http = http_client::native::NativeClient::new();
    // Disable detection entirely so we see the raw response instead of an error.
    let client = LastFmEditClientImpl::from_session_with_config(
        Box::new(http),
        session,
        lastfm_edit::RetryConfig::disabled(),
        lastfm_edit::RateLimitConfig::disabled(),
    );

    let url = std::env::var("PROBE_URL").unwrap_or_else(|_| {
        format!("{base_url}/user/{username}/library/edit?edited-variation=library-track-scrobble")
    });
    println!("GET {url}");
    let mut response = client.get(&url).await?;
    let status = response.status();
    println!("status: {status}");
    let body = response.body_string().await.map_err(|e| e.to_string())?;
    println!("body length: {}", body.len());
    let lower = body.to_lowercase();
    for p in PATTERNS {
        if let Some(idx) = lower.find(&p.to_lowercase()) {
            let start = idx.saturating_sub(120);
            let end = (idx + p.len() + 120).min(body.len());
            println!("\nMATCH pattern {p:?} at byte {idx}:");
            println!("...{}...", &body[start..end].replace('\n', " "));
        }
    }
    println!("first 400 bytes:\n{}", &body[..400.min(body.len())]);
    if let Some(idx) = lower.find("<title>") {
        let end = lower[idx..]
            .find("</title>")
            .map(|e| idx + e)
            .unwrap_or(idx + 80);
        println!("\ntitle: {}", &body[idx..end]);
    }
    Ok(())
}
