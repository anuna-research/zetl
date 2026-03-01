/// Minimal URL-encoding for page names (spaces → %20, etc.).
pub fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '#' => "%23".to_string(),
            '?' => "%3F".to_string(),
            '&' => "%26".to_string(),
            '%' => "%25".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

/// Escape HTML special characters.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
