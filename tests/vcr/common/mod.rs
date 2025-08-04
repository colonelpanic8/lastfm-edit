#![allow(dead_code)]
use http_client_vcr::{FilterChain, NoOpClient, VcrClient, VcrMode};
use lastfm_edit::vcr_test_utils::create_lastfm_test_filter_chain;
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};
use std::env;
use std::fs;

/// Shared setup for VCR test clients
struct VcrTestSetup {
    cassette_path: String,
    vcr_record: bool,
    mode: VcrMode,
}

impl VcrTestSetup {
    fn new(test_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let cassette_path = format!("tests/vcr/fixtures/{test_name}.yaml");

        // Ensure fixtures directory exists
        if let Some(parent_dir) = std::path::Path::new(&cassette_path).parent() {
            fs::create_dir_all(parent_dir)?;
        }

        let vcr_record = env::var("VCR_RECORD").is_ok();
        let cassette_exists = std::path::Path::new(&cassette_path).exists();

        // Fail fast if we're not recording and no cassette exists
        if !vcr_record && !cassette_exists {
            return Err(format!(
                "No cassette found at '{cassette_path}' and VCR_RECORD is not set. Either set VCR_RECORD to record new interactions or ensure the cassette file exists."
            ).into());
        }

        let mode = if vcr_record {
            VcrMode::Record
        } else {
            VcrMode::Replay
        };

        Ok(Self {
            cassette_path,
            vcr_record,
            mode,
        })
    }

    fn get_credentials(&self) -> (String, String) {
        if self.vcr_record {
            // Recording mode: need real credentials
            let username = env::var("LASTFM_EDIT_USERNAME")
                .expect("LASTFM_EDIT_USERNAME required when VCR_RECORD=true");
            let password = env::var("LASTFM_EDIT_PASSWORD")
                .expect("LASTFM_EDIT_PASSWORD required when VCR_RECORD=true");
            (username, password)
        } else {
            // Replay mode: use test credentials
            ("IvanMalison".to_string(), "test_password".to_string())
        }
    }

    async fn create_vcr_client(&self) -> Result<VcrClient, Box<dyn std::error::Error>> {
        let inner_client: Box<dyn http_client::HttpClient + Send + Sync> = if self.vcr_record {
            Box::new(http_client::native::NativeClient::new())
        } else {
            Box::new(NoOpClient::new())
        };

        let vcr_client = VcrClient::builder(&self.cassette_path)
            .inner_client(inner_client)
            .mode(self.mode.clone())
            .build()
            .await?;

        Ok(vcr_client)
    }

    async fn apply_filters_if_needed(&self) -> Result<(), Box<dyn std::error::Error>> {
        if matches!(self.mode, VcrMode::Record | VcrMode::Once) {
            let filter_chain = create_lastfm_test_filter_chain()?;
            http_client_vcr::filter_cassette_file(&self.cassette_path, filter_chain).await?;
        }
        Ok(())
    }
}

/// Helper for creating Last.fm VCR test clients that INCLUDES login in the VCR recording
/// Use this when you want to test the login process itself
pub async fn create_lastfm_vcr_test_client_with_login_recording(
    test_name: &str,
) -> Result<Box<dyn LastFmEditClient>, Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new(test_name)?;
    let (username, password) = setup.get_credentials();
    let vcr_client = setup.create_vcr_client().await?;

    // Login with VCR recording the interaction
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(vcr_client), &username, &password)
            .await
            .map_err(|e| {
                if setup.vcr_record {
                    format!("Recording mode login failed: {e}")
                } else {
                    format!("Replay mode login failed: {e}")
                }
            })?;

    setup.apply_filters_if_needed().await?;
    Ok(Box::new(client))
}

/// Helper for creating Last.fm VCR test clients that EXCLUDES login from VCR recording
/// Login happens outside VCR, only feature interactions are recorded
/// This is the default behavior for most tests
pub async fn create_lastfm_vcr_test_client_without_login_recording(
    test_name: &str,
) -> Result<Box<dyn LastFmEditClient>, Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new(test_name)?;

    if setup.vcr_record {
        // Recording mode: do real login outside VCR, then create VCR client with session
        let (username, password) = setup.get_credentials();

        // Do login outside VCR to get session
        let login_client = Box::new(http_client::native::NativeClient::new());
        let logged_in_client =
            LastFmEditClientImpl::login_with_credentials(login_client, &username, &password)
                .await
                .map_err(|e| format!("Login failed: {e}"))?;

        // Extract session from the logged-in client
        let session = logged_in_client.get_session().clone();

        // Now create VCR client for actual test interactions (using special method for recording)
        let vcr_client = VcrClient::builder(&setup.cassette_path)
            .inner_client(Box::new(http_client::native::NativeClient::new()))
            .mode(setup.mode.clone())
            .build()
            .await?;

        // Create client with existing session and VCR http client
        let client = LastFmEditClientImpl::from_session(Box::new(vcr_client), session);

        setup.apply_filters_if_needed().await?;
        Ok(Box::new(client))
    } else {
        // Replay mode: create dummy session and VCR client that will replay interactions
        let vcr_client = setup.create_vcr_client().await?;

        // Create a dummy session for replay mode
        let session = lastfm_edit::LastFmEditSession::new(
            "IvanMalison".to_string(),
            vec!["dummy_cookie".to_string()],
            Some("dummy_csrf".to_string()),
            "https://www.last.fm".to_string(),
        );

        let client = LastFmEditClientImpl::from_session(Box::new(vcr_client), session);
        Ok(Box::new(client))
    }
}

/// Helper for creating Last.fm VCR test clients that excludes login from recording
/// This is the default behavior - login happens outside VCR, only test interactions are recorded
pub async fn create_lastfm_vcr_test_client(
    test_name: &str,
) -> Result<Box<dyn LastFmEditClient>, Box<dyn std::error::Error>> {
    create_lastfm_vcr_test_client_without_login_recording(test_name).await
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
