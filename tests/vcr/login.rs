use super::common;

#[test_log::test(tokio::test)]
async fn login_test() {
    // Create VCR client that records login interaction for this test
    common::create_lastfm_vcr_test_client_with_login_recording("login_test")
        .await
        .expect("Client creation should succeed");
}
