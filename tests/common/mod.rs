#![allow(dead_code)]
use http_client_vcr::{FilterChain, NoOpClient, VcrClient, VcrMode};
use lastfm_edit::vcr_test_utils::create_lastfm_test_filter_chain;
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};
use std::env;
use std::fs;

/// Helper for creating Last.fm VCR test clients with proper setup
/// This handles the "login and then do stuff" pattern consistently across tests
pub async fn create_lastfm_vcr_test_client(
    test_name: &str,
) -> Result<Box<dyn LastFmEditClient>, Box<dyn std::error::Error>> {
    let cassette_path = format!("tests/fixtures/{test_name}.yaml");

    // Ensure fixtures directory exists
    if let Some(parent_dir) = std::path::Path::new(&cassette_path).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    let vcr_record = env::var("VCR_RECORD").is_ok();
    let cassette_exists = std::path::Path::new(&cassette_path).exists();

    let mode = if vcr_record {
        VcrMode::Record
    } else if cassette_exists {
        VcrMode::Replay
    } else {
        VcrMode::Once
    };

    // Handle credentials based on mode
    let (username, password) = if vcr_record {
        // Recording mode: need real credentials
        let username = env::var("LASTFM_EDIT_USERNAME")
            .expect("LASTFM_EDIT_USERNAME required when VCR_RECORD=true");
        let password = env::var("LASTFM_EDIT_PASSWORD")
            .expect("LASTFM_EDIT_PASSWORD required when VCR_RECORD=true");
        (username, password)
    } else {
        // Replay mode: use test credentials
        // The cassette should have the real username preserved but password filtered
        ("IvanMalison".to_string(), "test_password".to_string())
    };

    // Create VCR client - use NoOpClient when not recording to prevent real HTTP requests
    let inner_client: Box<dyn http_client::HttpClient + Send + Sync> = if vcr_record {
        Box::new(http_client::native::NativeClient::new())
    } else {
        Box::new(NoOpClient::new())
    };

    let vcr_client = VcrClient::builder(&cassette_path)
        .inner_client(inner_client)
        .mode(mode.clone())
        .build()
        .await?;

    // Attempt login
    let client_result =
        LastFmEditClientImpl::login_with_credentials(Box::new(vcr_client), &username, &password)
            .await;

    match client_result {
        Ok(client) => {
            // If we just recorded, apply filters to the cassette for future test runs
            if matches!(mode, VcrMode::Record | VcrMode::Once) {
                let filter_chain = create_lastfm_test_filter_chain()?;
                http_client_vcr::filter_cassette_file(&cassette_path, filter_chain).await?;
            }

            Ok(Box::new(client))
        }
        Err(e) => {
            if vcr_record {
                Err(format!("Recording mode login failed: {e}").into())
            } else {
                Err(format!("Replay mode login failed: {e}").into())
            }
        }
    }
}

pub async fn create_vcr_client(
    cassette_path: &str,
    mode: VcrMode,
    filter_chain: Option<FilterChain>,
) -> Result<VcrClient, Box<dyn std::error::Error>> {
    // Ensure fixtures directory exists
    if let Some(parent_dir) = std::path::Path::new(cassette_path).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    // Use NoOpClient when not in Record mode to prevent real HTTP requests
    let inner_client: Box<dyn http_client::HttpClient + Send + Sync> = match mode {
        VcrMode::Record => Box::new(http_client::native::NativeClient::new()),
        _ => Box::new(NoOpClient::new()),
    };
    let mut builder = VcrClient::builder(cassette_path)
        .inner_client(inner_client)
        .mode(mode);

    if let Some(filters) = filter_chain {
        builder = builder.filter_chain(filters);
    }

    Ok(builder.build().await?)
}
