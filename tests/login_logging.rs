use lastfm_edit::LoginManager;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct CapturedLogs(Mutex<String>);
static LOGS: CapturedLogs = CapturedLogs(Mutex::new(String::new()));

impl log::Log for CapturedLogs {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("lastfm_edit")
    }
    fn log(&self, record: &log::Record<'_>) {
        use std::fmt::Write;
        if self.enabled(record.metadata()) {
            writeln!(self.0.lock().unwrap(), "{}", record.args()).unwrap();
        }
    }
    fn flush(&self) {}
}

const PASSWORD: &str = "synthetic password&secret";
const CSRF: &str = "synthetic-csrf-secret";
const COOKIE: &str = "sessionid=.synthetic-session-secret-long-enough-for-login-validation";

#[derive(Debug)]
struct LoginResponses {
    calls: AtomicUsize,
    status: u16,
}

#[async_trait::async_trait]
impl http_client::HttpClient for LoginResponses {
    async fn send(
        &self,
        mut request: http_client::Request,
    ) -> http_types::Result<http_client::Response> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let mut response = http_types::Response::new(if first { 200 } else { self.status });
        if first {
            response.set_body(format!(
                "<input name=\"csrfmiddlewaretoken\" value=\"{CSRF}\">"
            ));
        } else {
            assert!(request
                .body_string()
                .await?
                .contains(&format!("password={}", urlencoding::encode(PASSWORD))));
            if self.status == 302 {
                response.insert_header("set-cookie", COOKIE)?;
            }
            response.set_body(format!(
                "<div class=\"alert-danger\">{PASSWORD}</div><input value=\"{CSRF}\">"
            ));
        }
        Ok(response)
    }
}

#[tokio::test]
async fn login_logs_never_contain_credentials_on_success_or_failure() {
    log::set_logger(&LOGS).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    for status in [302, 200, 403] {
        let transport = Arc::new(LoginResponses {
            calls: AtomicUsize::new(0),
            status,
        });
        let login = LoginManager::new(transport, "https://www.last.fm".into());
        let result = login.login("synthetic-user", PASSWORD).await;
        assert_eq!(result.is_ok(), status == 302);
    }
    let logs = LOGS.0.lock().unwrap();
    assert!(!logs.is_empty());
    for secret in [
        PASSWORD,
        urlencoding::encode(PASSWORD).as_ref(),
        CSRF,
        COOKIE,
    ] {
        assert!(!logs.contains(secret), "credential appeared in login logs");
    }
}
