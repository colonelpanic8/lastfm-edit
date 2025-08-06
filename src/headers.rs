use http_client::Request;

/// Common Chrome user agent string for all requests
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";

/// Common Chrome headers for security info
const SEC_CH_UA: &str =
    "\"Not)A;Brand\";v=\"8\", \"Chromium\";v=\"138\", \"Google Chrome\";v=\"138\"";
const SEC_CH_UA_MOBILE: &str = "?0";
const SEC_CH_UA_PLATFORM: &str = "\"Linux\"";

/// Add common browser headers to a request
pub fn add_common_headers(request: &mut Request) {
    let _ = request.insert_header("User-Agent", USER_AGENT);
    let _ = request.insert_header("Accept-Language", "en-US,en;q=0.9");
    let _ = request.insert_header("Accept-Encoding", "gzip, deflate, br");
    let _ = request.insert_header("DNT", "1");
    let _ = request.insert_header("Connection", "keep-alive");
    let _ = request.insert_header("sec-ch-ua", SEC_CH_UA);
    let _ = request.insert_header("sec-ch-ua-mobile", SEC_CH_UA_MOBILE);
    let _ = request.insert_header("sec-ch-ua-platform", SEC_CH_UA_PLATFORM);
}

/// Add headers for AJAX form edit requests
pub fn add_edit_headers(request: &mut Request, referer_url: &str) {
    add_common_headers(request);
    let _ = request.insert_header("Accept", "*/*");
    let _ = request.insert_header(
        "Content-Type",
        "application/x-www-form-urlencoded;charset=UTF-8",
    );
    let _ = request.insert_header("Priority", "u=1, i");
    let _ = request.insert_header("X-Requested-With", "XMLHttpRequest");
    let _ = request.insert_header("Sec-Fetch-Dest", "empty");
    let _ = request.insert_header("Sec-Fetch-Mode", "cors");
    let _ = request.insert_header("Sec-Fetch-Site", "same-origin");
    let _ = request.insert_header("Referer", referer_url);
}

/// Add headers for GET requests (regular pages or AJAX)
pub fn add_get_headers(request: &mut Request, is_ajax: bool, referer_url: Option<&str>) {
    add_common_headers(request);

    if is_ajax {
        let _ = request.insert_header("Accept", "*/*");
        let _ = request.insert_header("X-Requested-With", "XMLHttpRequest");
    } else {
        let _ = request.insert_header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
        );
        let _ = request.insert_header("Upgrade-Insecure-Requests", "1");
    }

    if let Some(referer) = referer_url {
        let _ = request.insert_header("Referer", referer);
    }
}

/// Add cookies to a request if they exist
pub fn add_cookies(request: &mut Request, cookies: &[String]) {
    if !cookies.is_empty() {
        let cookie_header = cookies.join("; ");
        let _ = request.insert_header("Cookie", &cookie_header);
    }
}
