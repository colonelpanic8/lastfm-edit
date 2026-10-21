use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use lastfm_edit::{
    ClientConfig, LastFmEditClientImpl, LastFmEditSession, LastFmError, ScrobblePageError,
};

const FORM: &str = r#"<table class="chartlist"><tr><td>
    <form data-edit-scrobble>
        <input name="track_name" value="Track">
        <input name="artist_name" value="Artist">
        <input name="album_name" value="Album">
        <input name="album_artist_name" value="Album Artist">
        <input name="timestamp" value="100">
    </form>
</td></tr></table>"#;
const EMPTY_CHARTLIST: &str = r#"<table class="chartlist"></table>"#;
const LOGGED_OUT: &str = r#"<a href="/login?next=/user/test/library">Log in</a>"#;
const NEXT_PAGE: &str = r#"<div class="pagination"><a class="pagination-next">Next</a></div>"#;

#[derive(Debug)]
struct ScriptedClient {
    bodies: Mutex<VecDeque<String>>,
    requests: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl http_client::HttpClient for ScriptedClient {
    async fn send(
        &self,
        request: http_client::Request,
    ) -> std::result::Result<http_client::Response, http_types::Error> {
        self.requests
            .lock()
            .unwrap()
            .push(request.url().to_string());
        let body = self
            .bodies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected request");
        let mut response = http_types::Response::new(200);
        response.set_body(body);
        Ok(response)
    }
}

fn client(bodies: Vec<String>) -> (LastFmEditClientImpl, Arc<Mutex<Vec<String>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = ScriptedClient {
        bodies: Mutex::new(bodies.into()),
        requests: requests.clone(),
    };
    let session = LastFmEditSession::new(
        "test".into(),
        vec!["sessionid=test".into()],
        Some("csrf".into()),
        "https://www.last.fm".into(),
    );
    (
        LastFmEditClientImpl::from_session_with_client_config(
            Box::new(transport),
            session,
            ClientConfig::for_testing(),
        ),
        requests,
    )
}

fn assert_page_error(error: LastFmError, expected: ScrobblePageError, expected_url: &str) {
    assert!(std::error::Error::source(&error).is_some());
    match error {
        LastFmError::ScrobblePage {
            track_name,
            artist_name,
            url,
            source,
        } => {
            assert_eq!(track_name, "Track");
            assert_eq!(artist_name, "Artist");
            assert_eq!(url, expected_url);
            assert_eq!(source, expected);
        }
        other => panic!("expected structured track-page error, got {other:?}"),
    }
}

#[tokio::test]
async fn exhausted_candidates_report_typed_failure_and_context() {
    for (html, reason) in [
        ("<html></html>", ScrobblePageError::MissingChartlist),
        (EMPTY_CHARTLIST, ScrobblePageError::MissingEditForms),
        (LOGGED_OUT, ScrobblePageError::LoggedOut),
    ] {
        let (client, requests) = client(vec![html.to_owned(); 6]);
        let error = client
            .get_scrobble_edit_variations("Track", "Artist")
            .await
            .unwrap_err();
        assert_page_error(
            error,
            reason,
            "https://www.last.fm/user/test/library/music/Artist/_/Track?ajax=1",
        );
        assert_eq!(requests.lock().unwrap().len(), 6);
    }
}

#[tokio::test]
async fn discovery_falls_back_to_ajax_and_preserves_album_artist() {
    let (client, requests) = client(vec![
        "<table class='chartlist chartlist__placeholder'></table>".into(),
        EMPTY_CHARTLIST.into(),
        FORM.into(),
    ]);
    let edits = client
        .get_scrobble_edit_variations("Track", "Artist")
        .await
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].album_artist_name_original, "Album Artist");
    assert_eq!(
        *requests.lock().unwrap(),
        [
            "https://www.last.fm/user/test/library/music/+noredirect/Artist/_/Track",
            "https://www.last.fm/user/test/library/music/Artist/_/Track",
            "https://www.last.fm/user/test/library/music/+noredirect/Artist/_/Track?ajax=true",
        ]
    );
}

#[tokio::test]
async fn pagination_keeps_ajax_variant_and_deduplicates_albums() {
    let (client, requests) = client(vec![
        "<html></html>".into(),
        EMPTY_CHARTLIST.into(),
        format!("{FORM}{NEXT_PAGE}"),
        FORM.into(),
    ]);
    let edits = client
        .get_scrobble_edit_variations("Track", "Artist")
        .await
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(
        requests.lock().unwrap().last().unwrap(),
        "https://www.last.fm/user/test/library/music/+noredirect/Artist/_/Track?page=2&ajax=true"
    );
}

#[tokio::test]
async fn session_expiry_during_pagination_is_not_partial_success() {
    for html in [
        LOGGED_OUT.to_owned(),
        format!("{EMPTY_CHARTLIST}{LOGGED_OUT}"),
    ] {
        let (client, _) = client(vec![format!("{FORM}{NEXT_PAGE}"), html]);
        let error = client
            .get_scrobble_edit_variations("Track", "Artist")
            .await
            .unwrap_err();
        assert_page_error(
            error,
            ScrobblePageError::LoggedOut,
            "https://www.last.fm/user/test/library/music/+noredirect/Artist/_/Track?page=2",
        );
    }
}

#[tokio::test]
async fn missing_chartlist_during_pagination_has_page_context() {
    let (client, _) = client(vec![format!("{FORM}{NEXT_PAGE}"), "<html></html>".into()]);
    let error = client
        .get_scrobble_edit_variations("Track", "Artist")
        .await
        .unwrap_err();
    assert_page_error(
        error,
        ScrobblePageError::MissingChartlist,
        "https://www.last.fm/user/test/library/music/+noredirect/Artist/_/Track?page=2",
    );
}
