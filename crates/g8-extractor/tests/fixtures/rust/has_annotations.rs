// PASS-detect fixture: Rust file with G8 magic-comment annotations.

// @g8.capability(name = "http-fetch", status = "in_flight", substrate = "net")
pub fn fetch_url(url: &str) -> Vec<u8> {
    vec![]
}

// @g8.capability(name = "json-parse", status = "landed", substrate = "data")
pub fn parse_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::Value::Null
}

// @g8.convergence_test(for_capability = "http-fetch", scenario = "timeout")
#[test]
fn test_http_fetch_timeout() {}

// @g8.intent(description = "HTTP networking utilities", substrate = "net")
mod net {}

// @g8.plan(title = "streaming-export", status = "Idea", substrate = "net")
fn placeholder_streaming_export() {}

// @g8.decision(title = "Use reqwest for HTTP", status = "accepted")
fn placeholder_decision() {}
