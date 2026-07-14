//! `http-client` adapter backed by reqwest's Rustls transport.
//!
//! The app uses this instead of the curl-backed native client so the same
//! transport works in Android builds. Reqwest performs content decoding before
//! the response is converted back to `http_types`.

use http_client::http_types::{self, StatusCode};
use http_client::{HttpClient, Request, Response};

#[derive(Clone, Debug)]
pub(crate) struct RustlsReqwestClient {
    client: reqwest::Client,
}

impl RustlsReqwestClient {
    pub(crate) fn new() -> Result<Self, String> {
        reqwest::Client::builder()
            .gzip(true)
            .brotli(true)
            .deflate(true)
            // The lastfm-edit login flow needs the original 302 response so it
            // can capture Set-Cookie before making the next request itself.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(|client| Self { client })
            .map_err(|error| format!("could not initialize HTTP client: {error}"))
    }
}

#[http_client::async_trait]
impl HttpClient for RustlsReqwestClient {
    async fn send(&self, mut request: Request) -> http_types::Result<Response> {
        let method = reqwest::Method::from_bytes(request.method().as_ref().as_bytes())
            .map_err(http_error)?;
        let mut outgoing = self.client.request(method, request.url().as_str());

        for (name, values) in request.iter() {
            for value in values.iter() {
                outgoing = outgoing.header(name.as_str(), value.as_str());
            }
        }

        let body = request.body_bytes().await?;
        if !body.is_empty() {
            outgoing = outgoing.body(body);
        }

        let incoming = outgoing.send().await.map_err(http_error)?;
        let status = incoming.status().as_u16();
        let headers = incoming.headers().clone();
        let version = match incoming.version() {
            reqwest::Version::HTTP_09 => Some(http_types::Version::Http0_9),
            reqwest::Version::HTTP_10 => Some(http_types::Version::Http1_0),
            reqwest::Version::HTTP_11 => Some(http_types::Version::Http1_1),
            reqwest::Version::HTTP_2 => Some(http_types::Version::Http2_0),
            reqwest::Version::HTTP_3 => Some(http_types::Version::Http3_0),
            _ => None,
        };
        let body = incoming.bytes().await.map_err(http_error)?;

        let mut response = Response::new(status);
        response.set_version(version);
        for (name, value) in &headers {
            let value = value.to_str().map_err(http_error)?;
            response.append_header(name.as_str(), value)?;
        }
        response.set_body(body.to_vec());
        Ok(response)
    }
}

fn http_error(error: impl std::fmt::Display) -> http_types::Error {
    http_types::Error::from_str(StatusCode::InternalServerError, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_client::http_types::Url;
    use std::io::Write;
    use std::net::TcpListener;

    #[tokio::test(flavor = "current_thread")]
    async fn transparently_decodes_gzip_responses() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"decoded response").unwrap();
        let encoded = encoder.finish().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};

            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                encoded.len()
            )
            .unwrap();
            stream.write_all(&encoded).unwrap();
        });

        let client = RustlsReqwestClient::new().unwrap();
        let request = Request::new(
            http_types::Method::Get,
            Url::parse(&format!("http://{address}/compressed")).unwrap(),
        );
        let mut response = client.send(request).await.unwrap();

        assert_eq!(response.body_string().await.unwrap(), "decoded response");
        assert!(response.header("content-encoding").is_none());
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn preserves_redirect_responses_for_login_cookie_capture() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};

            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /account\r\nSet-Cookie: sessionid=.mobile-session; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let client = RustlsReqwestClient::new().unwrap();
        let request = Request::new(
            http_types::Method::Post,
            Url::parse(&format!("http://{address}/login")).unwrap(),
        );
        let response = client.send(request).await.unwrap();

        assert_eq!(response.status(), http_types::StatusCode::Found);
        assert_eq!(response["location"].as_str(), "/account");
        assert!(response["set-cookie"]
            .as_str()
            .starts_with("sessionid=.mobile-session"));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires live Last.fm credentials"]
    async fn live_lastfm_login_works_with_mobile_transport() {
        let username = std::env::var("LASTFM_EDIT_USERNAME")
            .expect("LASTFM_EDIT_USERNAME must be set for the live login test");
        let password = std::env::var("LASTFM_EDIT_PASSWORD")
            .expect("LASTFM_EDIT_PASSWORD must be set for the live login test");

        lastfm_edit::LastFmEditClientImpl::login_with_credentials(
            Box::new(RustlsReqwestClient::new().unwrap()),
            &username,
            &password,
        )
        .await
        .expect("the mobile transport should authenticate with Last.fm");
    }
}
