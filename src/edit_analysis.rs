use http_types::StatusCode;
use scraper::{Html, Selector};

/// Result of analyzing an edit response from Last.fm
#[derive(Debug, Clone)]
pub struct EditAnalysisResult {
    /// Whether the edit was successful based on all indicators
    pub success: bool,
    /// Optional detailed message about the result
    pub message: Option<String>,
    /// Track name found in the response (if any)
    pub actual_track_name: Option<String>,
    /// Album name found in the response (if any)
    pub actual_album_name: Option<String>,
}

/// Analyze the HTML response from a Last.fm edit request to determine success/failure
///
/// This function parses the response HTML to look for success/error indicators
/// and extract the actual track/album names that were processed.
///
/// # Arguments
/// * `response_text` - The HTML response body from the edit request
/// * `status_code` - The HTTP status code of the response
///
/// # Returns
/// An `EditAnalysisResult` containing the analysis results
pub fn analyze_edit_response(response_text: &str, status_code: StatusCode) -> EditAnalysisResult {
    // Parse the HTML response to check for actual success/failure
    let document = Html::parse_document(response_text);

    // Check for success indicator
    let success_selector = Selector::parse(".alert-success").unwrap();
    let error_selector = Selector::parse(".alert-danger, .alert-error, .error").unwrap();

    let has_success_alert = document.select(&success_selector).next().is_some();
    let has_error_alert = document.select(&error_selector).next().is_some();

    // Extract track and album names from the response
    let (actual_track_name, actual_album_name) =
        extract_track_album_names(&document, response_text);

    log::debug!(
        "Response analysis: success_alert={}, error_alert={}, track='{}', album='{}'",
        has_success_alert,
        has_error_alert,
        actual_track_name.as_deref().unwrap_or("not found"),
        actual_album_name.as_deref().unwrap_or("not found")
    );

    // Determine if edit was truly successful
    let final_success = status_code.is_success() && has_success_alert && !has_error_alert;

    // Create detailed message
    let message = if has_error_alert {
        // Extract error message
        if let Some(error_element) = document.select(&error_selector).next() {
            Some(format!(
                "Edit failed: {}",
                error_element.text().collect::<String>().trim()
            ))
        } else {
            Some("Edit failed with unknown error".to_string())
        }
    } else if final_success {
        Some(format!(
            "Edit successful - Track: '{}', Album: '{}'",
            actual_track_name.as_deref().unwrap_or("unknown"),
            actual_album_name.as_deref().unwrap_or("unknown")
        ))
    } else {
        Some(format!("Edit failed with status: {status_code}"))
    };

    EditAnalysisResult {
        success: final_success,
        message,
        actual_track_name,
        actual_album_name,
    }
}

/// Extract track and album names from the edit response
///
/// This function tries multiple strategies to find the actual track and album names
/// in the response, including direct CSS selectors and regex patterns.
fn extract_track_album_names(
    document: &Html,
    response_text: &str,
) -> (Option<String>, Option<String>) {
    let mut actual_track_name = None;
    let mut actual_album_name = None;

    // Try direct selectors first
    let track_name_selector = Selector::parse("td.chartlist-name a").unwrap();
    let album_name_selector = Selector::parse("td.chartlist-album a").unwrap();

    if let Some(track_element) = document.select(&track_name_selector).next() {
        actual_track_name = Some(track_element.text().collect::<String>().trim().to_string());
    }

    if let Some(album_element) = document.select(&album_name_selector).next() {
        actual_album_name = Some(album_element.text().collect::<String>().trim().to_string());
    }

    // If not found, try extracting from the raw response text using generic patterns
    if actual_track_name.is_none() || actual_album_name.is_none() {
        if actual_track_name.is_none() {
            actual_track_name = extract_track_name_from_text(response_text);
        }

        if actual_album_name.is_none() {
            actual_album_name = extract_album_name_from_text(response_text);
        }
    }

    (actual_track_name, actual_album_name)
}

/// Extract track name from response text using regex patterns
fn extract_track_name_from_text(response_text: &str) -> Option<String> {
    // Look for track name in href="/music/{artist}/_/{track}"
    // Use regex to find track URLs
    let track_pattern = regex::Regex::new(r#"href="/music/[^"]+/_/([^"]+)""#).unwrap();
    if let Some(captures) = track_pattern.captures(response_text) {
        if let Some(track_match) = captures.get(1) {
            let raw_track = track_match.as_str();
            // URL decode the track name
            let decoded_track = urlencoding::decode(raw_track)
                .unwrap_or_else(|_| raw_track.into())
                .replace('+', " ");
            return Some(decoded_track);
        }
    }
    None
}

/// Extract album name from response text using regex patterns
fn extract_album_name_from_text(response_text: &str) -> Option<String> {
    // Look for album name in href="/music/{artist}/{album}"
    // Find album links that are not track links (don't contain /_/)
    let album_pattern =
        regex::Regex::new(r#"href="/music/[^"]+/([^"/_]+)"[^>]*>[^<]*</a>"#).unwrap();
    if let Some(captures) = album_pattern.captures(response_text) {
        if let Some(album_match) = captures.get(1) {
            let raw_album = album_match.as_str();
            // URL decode the album name
            let decoded_album = urlencoding::decode(raw_album)
                .unwrap_or_else(|_| raw_album.into())
                .replace('+', " ");
            return Some(decoded_album);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_success_response() {
        let html = r#"
            <div class="alert-success">Edit successful</div>
            <table>
                <tr>
                    <td class="chartlist-name"><a href="/music/artist/_/track">Test Track</a></td>
                    <td class="chartlist-album"><a href="/music/artist/album">Test Album</a></td>
                </tr>
            </table>
        "#;

        let result = analyze_edit_response(html, StatusCode::Ok);
        assert!(result.success);
        // The CSS selectors should extract the text content of the links
        assert_eq!(result.actual_track_name, Some("Test Track".to_string()));
        assert_eq!(result.actual_album_name, Some("Test Album".to_string()));
    }

    #[test]
    fn test_analyze_error_response() {
        let html = r#"
            <div class="alert-danger">Edit failed: Invalid data</div>
        "#;

        let result = analyze_edit_response(html, StatusCode::Ok);
        assert!(!result.success);
        assert!(result
            .message
            .unwrap()
            .contains("Edit failed: Invalid data"));
    }

    #[test]
    fn test_extract_from_regex_patterns() {
        let html = r#"
            Some content with <a href="/music/Artist/AlbumName">album link</a>
            and later <a href="/music/Artist/_/TrackName">track link</a>
        "#;

        let result = analyze_edit_response(html, StatusCode::Ok);
        // Should extract from regex patterns when direct selectors fail
        // The track pattern captures from /_/ URLs, album pattern from non-/_/ URLs
        assert_eq!(result.actual_track_name, Some("TrackName".to_string()));
        assert_eq!(result.actual_album_name, Some("AlbumName".to_string()));
    }
}
