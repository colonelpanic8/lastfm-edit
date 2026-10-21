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
#[path = "http_tests.rs"]
mod tests;
