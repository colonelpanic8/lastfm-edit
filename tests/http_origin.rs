use lastfm_edit::{ClientConfig, LastFmEditClientImpl, LastFmEditSession};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct RedirectClient {
    target: String,
    requests: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl http_client::HttpClient for RedirectClient {
    async fn send(
        &self,
        request: http_client::Request,
    ) -> http_types::Result<http_client::Response> {
        let mut requests = self.requests.lock().unwrap();
        assert_eq!(
            request.header("cookie").unwrap().as_str(),
            "sessionid=synthetic"
        );
        requests.push(request.url().to_string());
        let mut response = http_types::Response::new(if requests.len() == 1 { 302 } else { 200 });
        if requests.len() == 1 {
            response.insert_header("Location", &self.target)?;
        }
        response.set_body("ok");
        Ok(response)
    }
}

fn client(target: &str) -> (LastFmEditClientImpl, Arc<Mutex<Vec<String>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let client = LastFmEditClientImpl::from_session_with_client_config(
        Box::new(RedirectClient {
            target: target.into(),
            requests: requests.clone(),
        }),
        LastFmEditSession::new(
            "test".into(),
            vec!["sessionid=synthetic".into()],
            None,
            "https://www.last.fm".into(),
        ),
        ClientConfig::for_testing(),
    );
    (client, requests)
}

#[tokio::test]
async fn refuses_cross_origin_redirects_and_https_downgrades_before_sending() {
    for target in [
        "https://other.example/landing",
        "//other.example/landing",
        "http://www.last.fm/landing",
        "https://www.last.fm:444/landing",
    ] {
        let (client, requests) = client(target);
        assert!(client.get("https://www.last.fm/start").await.is_err());
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn refuses_external_and_invalid_initial_urls() {
    for url in ["https://other.example", "http://www.last.fm", "not a url"] {
        let (client, requests) = client("/unused");
        assert!(client.get(url).await.is_err());
        assert!(requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn follows_same_origin_redirects_with_cookies() {
    for target in ["../landing", "/landing", "https://www.last.fm/landing"] {
        let (client, requests) = client(target);
        assert!(client.get("https://www.last.fm/path/start").await.is_ok());
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [
                "https://www.last.fm/path/start",
                "https://www.last.fm/landing"
            ]
        );
    }
}
