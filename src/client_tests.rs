use super::*;

/// Captured copy of the "Last.fm - Rate Limited" interstitial that last.fm serves
/// (with a non-success HTTP status) when it throttles an account.
const RATE_LIMITED_PAGE: &str = include_str!("../tests/fixtures/rate_limited_page.html");

/// HTTP client stub for tests that never issue requests.
#[derive(Debug)]
struct NoopHttpClient;

#[async_trait::async_trait]
impl HttpClient for NoopHttpClient {
    async fn send(
        &self,
        _req: http_client::Request,
    ) -> std::result::Result<http_client::Response, http_types::Error> {
        Ok(http_types::Response::new(200))
    }
}

fn test_client(config: ClientConfig) -> LastFmEditClientImpl {
    let session = LastFmEditSession::new(
        "test_user".to_string(),
        vec!["sessionid=.test_session_id".to_string()],
        Some("test_csrf_token".to_string()),
        "https://www.last.fm".to_string(),
    );
    LastFmEditClientImpl::from_session_with_client_config(Box::new(NoopHttpClient), session, config)
}

#[test]
fn is_rate_limit_response_matches_captured_rate_limited_page() {
    let client = test_client(ClientConfig::default());
    assert!(client.is_rate_limit_response(RATE_LIMITED_PAGE));
}

#[test]
fn page_indicates_logged_out_detects_login_links() {
    // Logged-out library pages route actions through /login?next=...
    assert!(page_indicates_logged_out(
        r#"<a href="/login?next=/user/someone/library">Set track as current obsession</a>"#
    ));
    // A logged-in page with edit forms has no such link.
    assert!(!page_indicates_logged_out(
        r#"<form method="POST" data-edit-scrobble><input name="track_name"></form>"#
    ));
}

/// HTTP client stub that panics inside `send`, like http-client's isahc adapter
/// does when a server returns a status code outside http-types' enum.
#[derive(Debug)]
struct PanickingHttpClient;

#[async_trait::async_trait]
impl HttpClient for PanickingHttpClient {
    async fn send(
        &self,
        _req: http_client::Request,
    ) -> std::result::Result<http_client::Response, http_types::Error> {
        panic!("Could not convert into a valid `StatusCode`: Invalid status code");
    }
}

#[tokio::test]
async fn guard_client_turns_send_panics_into_errors() {
    let client = guard_client(Box::new(PanickingHttpClient));
    let request = Request::new(Method::Get, "https://www.last.fm/".parse::<Url>().unwrap());
    let err = client
        .send(request)
        .await
        .expect_err("panic must become an error");
    assert!(
        err.to_string().contains("HTTP client panicked"),
        "unexpected error: {err}"
    );
}

#[test]
fn is_rate_limit_response_ignores_normal_page() {
    let client = test_client(ClientConfig::default());
    assert!(!client.is_rate_limit_response(
        "<html><head><title>Music | Last.fm</title></head><body>ok</body></html>"
    ));
}

#[test]
fn response_indicates_rate_limit_scopes_pattern_detection_to_non_success() {
    let client = test_client(ClientConfig::default());

    // 200 OK whose body contains a pattern phrase (as a track title) is NOT rate limited.
    let ok_body = "<html><body><a class=\"track\">Slow Down</a> by Douglas</body></html>";
    assert!(!client.response_indicates_rate_limit(http_types::StatusCode::Ok, ok_body));

    // 503 serving the captured interstitial IS rate limited.
    assert!(client.response_indicates_rate_limit(
        http_types::StatusCode::ServiceUnavailable,
        RATE_LIMITED_PAGE
    ));

    // 403 with a plain body is NOT rate limited.
    assert!(!client.response_indicates_rate_limit(
        http_types::StatusCode::Forbidden,
        "<html><body>Forbidden</body></html>"
    ));

    // 403 whose body matches a rate-limit pattern IS rate limited.
    assert!(client.response_indicates_rate_limit(
        http_types::StatusCode::Forbidden,
        "<html><body>you're requesting too many pages</body></html>"
    ));
}

#[test]
fn response_indicates_rate_limit_detects_200_status_interstitial() {
    let client = test_client(ClientConfig::default());

    // 200 OK whose body contains a track literally named "Slow Down" is NOT rate limited:
    // the narrow success-pattern tier must not collide with library content.
    let track_body = "<html><body><a class=\"track\">Slow Down</a> by The Beatles</body></html>";
    assert!(!client.response_indicates_rate_limit(http_types::StatusCode::Ok, track_body));

    // 200 OK serving the real soft-throttle interstitial sentence IS rate limited.
    let interstitial_body = "<html><body><p>You're requesting a lot of pages which may \
             slow down the site for other users</p></body></html>";
    assert!(client.response_indicates_rate_limit(http_types::StatusCode::Ok, interstitial_body));
}

#[test]
fn post_rate_limit_guard_rejects_406_interstitial_and_passes_normal_responses() {
    let client = test_client(ClientConfig::default());
    let interstitial = "<html><head><title>Last.fm - Rate Limited</title></head></html>";

    // 406 interstitial (as served site-wide after edit bursts) yields a RateLimit error
    // instead of flowing on to edit analysis / delete interpretation.
    let err = client
        .check_post_response_for_rate_limit(
            "https://www.last.fm/user/x/library/edit",
            http_types::StatusCode::NotAcceptable,
            interstitial,
        )
        .unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { retry_after: 60 }));

    // A normal edit response body passes through untouched.
    assert!(client
        .check_post_response_for_rate_limit(
            "https://www.last.fm/user/x/library/edit",
            http_types::StatusCode::Ok,
            "<div class=\"alert-success\">Your edit was saved</div>",
        )
        .is_ok());
}
