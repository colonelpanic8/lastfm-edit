use std::collections::HashMap;

/// Parse URL-encoded form data into key-value pairs
pub fn parse_form_data(data: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    for pair in data.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            // URL decode the key and value
            let decoded_key = urlencoding::decode(key).unwrap_or_else(|_| key.into());
            let decoded_value = urlencoding::decode(value).unwrap_or_else(|_| value.into());
            params.insert(decoded_key.to_string(), decoded_value.to_string());
        }
    }

    params
}

/// Encode form data back to URL-encoded string
pub fn encode_form_data(params: &HashMap<String, String>) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
