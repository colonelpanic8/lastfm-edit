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
