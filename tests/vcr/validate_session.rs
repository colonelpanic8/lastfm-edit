use super::common;

#[test_log::test(tokio::test)]
async fn test_validate_session() {
    let client = common::create_lastfm_vcr_test_client("validate_session")
        .await
        .expect("Failed to setup VCR client");

    // Test that session validation works for a valid session
    let is_valid = client.validate_session().await;
    assert!(
        is_valid,
        "Session should be valid for properly authenticated client"
    );
}
