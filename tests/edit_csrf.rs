//! With `reuse_csrf_token`, edits reuse the session's CSRF token and refresh it only when
//! Last.fm rejects it.

use lastfm_edit::{ClientConfig, ExactScrobbleEdit, LastFmEditClientImpl, LastFmEditSession};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const CURRENT_TOKEN: &str = "current-token";

#[derive(Debug, Default)]
struct Requests {
    gets: AtomicUsize,
    posts: AtomicUsize,
}

/// Serves the edit form with `CURRENT_TOKEN` and accepts only POSTs carrying it.
#[derive(Debug)]
struct CsrfCheckingLastFm(Arc<Requests>);

#[async_trait::async_trait]
impl http_client::HttpClient for CsrfCheckingLastFm {
    async fn send(
        &self,
        mut req: http_client::Request,
    ) -> std::result::Result<http_client::Response, http_types::Error> {
        if req.method() == http_types::Method::Get {
            self.0.gets.fetch_add(1, Ordering::SeqCst);
            let mut response = http_types::Response::new(200);
            response.set_body(format!(
                "<form><input name=\"csrfmiddlewaretoken\" value=\"{CURRENT_TOKEN}\"></form>"
            ));
            return Ok(response);
        }
        self.0.posts.fetch_add(1, Ordering::SeqCst);
        let body = req.body_string().await?;
        let mut response = if body.contains(&format!("csrfmiddlewaretoken={CURRENT_TOKEN}")) {
            let mut response = http_types::Response::new(200);
            response.set_body("<div class=\"alert-success\">Scrobble edited</div>");
            response
        } else {
            http_types::Response::new(403)
        };
        if response.status() == 403 {
            response.set_body("CSRF verification failed");
        }
        Ok(response)
    }
}

fn client(session_token: &str) -> (LastFmEditClientImpl, Arc<Requests>) {
    let requests = Arc::new(Requests::default());
    let session = LastFmEditSession::new(
        "test_user".to_string(),
        vec!["sessionid=.test_session_id_12345".to_string()],
        Some(session_token.to_string()),
        "https://www.last.fm".to_string(),
    );
    let client = LastFmEditClientImpl::from_session_with_client_config(
        Box::new(CsrfCheckingLastFm(requests.clone())),
        session,
        ClientConfig::with_retries_disabled().with_reuse_csrf_token(true),
    );
    (client, requests)
}

fn edit() -> ExactScrobbleEdit {
    ExactScrobbleEdit::new(
        "Track".to_string(),
        "Album".to_string(),
        "Artist".to_string(),
        "Artist".to_string(),
        "Track (Clean)".to_string(),
        "Album".to_string(),
        "Artist".to_string(),
        "Artist".to_string(),
        1_700_000_000,
        false,
    )
}

#[tokio::test]
async fn a_valid_session_token_edits_without_loading_the_form() {
    let (client, requests) = client(CURRENT_TOKEN);
    let response = client.edit_scrobble_single(&edit(), 0).await.unwrap();
    assert!(response.all_successful());
    assert_eq!(requests.gets.load(Ordering::SeqCst), 0);
    assert_eq!(requests.posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_stale_token_is_refreshed_once_and_then_reused() {
    let (client, requests) = client("token-from-login-page");
    assert!(client
        .edit_scrobble_single(&edit(), 0)
        .await
        .unwrap()
        .all_successful());
    assert_eq!(requests.gets.load(Ordering::SeqCst), 1);
    assert_eq!(requests.posts.load(Ordering::SeqCst), 2);

    assert!(client
        .edit_scrobble_single(&edit(), 0)
        .await
        .unwrap()
        .all_successful());
    assert_eq!(requests.gets.load(Ordering::SeqCst), 1);
    assert_eq!(requests.posts.load(Ordering::SeqCst), 3);
}
