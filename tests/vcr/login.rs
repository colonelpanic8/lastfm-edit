use super::common;

#[test_log::test(tokio::test)]
async fn login() {
    common::create_lastfm_vcr_test_client_with_login_recording("login")
        .await
        .expect("Client creation should succeed");
}
