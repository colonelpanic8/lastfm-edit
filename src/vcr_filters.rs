use http_client_vcr::{BodyFilter, FilterChain, HeaderFilter, SmartFormFilter};

/// Creates a filter chain specifically designed for Last.fm HTTP interactions
/// This removes/replaces sensitive authentication and session data using smart form parsing
pub fn create_lastfm_filter_chain() -> Result<FilterChain, regex::Error> {
    let filter_chain = FilterChain::new()
        // Filter sensitive headers
        .add_filter(Box::new(
            HeaderFilter::new()
                .replace_header("cookie", "[FILTERED_COOKIE]")
                .replace_header("set-cookie", "[FILTERED_SET-COOKIE]")
                .replace_header("authorization", "[FILTERED_AUTH]"),
        ))
        // Smart form data filtering - automatically detects and filters credentials
        .add_filter(Box::new(
            SmartFormFilter::new().with_replacement_pattern("[FILTERED_LASTFM]"),
        ))
        // Fallback regex filtering for form data and HTML content
        .add_filter(Box::new(
            BodyFilter::new()
                // CSRF tokens in HTML hidden form fields
                .replace_regex(
                    r#"name="csrfmiddlewaretoken"\s+value="[^"]+""#,
                    r#"name="csrfmiddlewaretoken" value="[FILTERED_CSRF]""#,
                )?
                // CSRF tokens in different HTML formats
                .replace_regex(
                    r#"<input[^>]*name=["']csrfmiddlewaretoken["'][^>]*value=["'][^"']+["'][^>]*>"#,
                    r#"<input type="hidden" name="csrfmiddlewaretoken" value="[FILTERED_CSRF]">"#,
                )?
                // Session IDs in cookies within HTML/JavaScript
                .replace_regex(r"sessionid=[^;,\s]+", "sessionid=[FILTERED_SESSION]")?
                // CSRF tokens in cookies within HTML/JavaScript
                .replace_regex(r"csrftoken=[^;,\s]+", "csrftoken=[FILTERED_CSRF]")?,
        ));

    Ok(filter_chain)
}

/// Creates a minimal filter chain that only filters the most critical sensitive data
/// Useful when you need to preserve more structure but still remove credentials
pub fn create_minimal_lastfm_filter_chain() -> Result<FilterChain, regex::Error> {
    let filter_chain = FilterChain::new()
        // Only filter the most sensitive headers
        .add_filter(Box::new(
            HeaderFilter::new().replace_header("set-cookie", "[FILTERED]"),
        ))
        // Smart form filtering with minimal replacement pattern
        .add_filter(Box::new(
            SmartFormFilter::new().with_replacement_pattern("[FILTERED]"),
        ));

    Ok(filter_chain)
}

/// Creates an aggressive filter chain that removes/masks all potentially sensitive data
/// Use this when maximum privacy is required
pub fn create_aggressive_lastfm_filter_chain() -> Result<FilterChain, regex::Error> {
    let filter_chain = FilterChain::new()
        // Filter all authentication-related headers
        .add_filter(Box::new(
            HeaderFilter::new()
                .remove_auth_headers() // Built-in method that removes common auth headers
                .replace_header("referer", "[FILTERED_REFERER]")
                .replace_header("user-agent", "[FILTERED_USER_AGENT]"),
        ))
        // Filter all form data and session-related content
        .add_filter(Box::new(
            BodyFilter::new()
                // All form fields that might contain sensitive data
                .replace_regex(
                    r"csrfmiddlewaretoken=[^&\s]+",
                    "csrfmiddlewaretoken=[FILTERED]",
                )?
                .replace_regex(r"username_or_email=[^&\s]+", "username_or_email=[FILTERED]")?
                .replace_regex(r"password=[^&\s]+", "password=[FILTERED]")?
                .replace_regex(r"sessionid=[^&\s;,]+", "sessionid=[FILTERED]")?
                .replace_regex(r"csrftoken=[^&\s;,]+", "csrftoken=[FILTERED]")?
                // Email patterns in any format
                .replace_regex(
                    r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b",
                    "[FILTERED_EMAIL]",
                )?
                // Any token-like strings (long alphanumeric sequences that might be sensitive)
                .replace_regex(r"\b[A-Za-z0-9]{20,}\b", "[FILTERED_TOKEN]")?,
        ));

    Ok(filter_chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_lastfm_filter_chain() {
        let result = create_lastfm_filter_chain();
        assert!(result.is_ok(), "Should create filter chain without errors");
    }

    #[test]
    fn test_create_minimal_lastfm_filter_chain() {
        let result = create_minimal_lastfm_filter_chain();
        assert!(
            result.is_ok(),
            "Should create minimal filter chain without errors"
        );
    }

    #[test]
    fn test_create_aggressive_lastfm_filter_chain() {
        let result = create_aggressive_lastfm_filter_chain();
        assert!(
            result.is_ok(),
            "Should create aggressive filter chain without errors"
        );
    }
}
