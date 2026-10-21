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
