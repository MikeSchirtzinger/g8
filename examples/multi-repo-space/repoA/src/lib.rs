//! repoA — API gateway service.
//!
//! Annotated with G8 magic-comments to demonstrate cross-repo conflict
//! detection via `g8 merge`.

// @g8.capability(name = "http-fetch", description = "Outbound HTTP client for upstream service calls", substrate = "http-layer", consumes = ["substrate::HttpRequest"], produces = ["substrate::HttpResponse"], status = "landed")
/// Perform an outbound HTTP GET request.
pub fn http_fetch(url: &str) -> Result<Vec<u8>, String> {
    if url.is_empty() {
        return Err("empty URL".into());
    }
    Ok(format!("response from {url}").into_bytes())
}

// @g8.convergence_test(for_capability = "http-fetch", scenario = "empty URL returns error")
#[test]
fn test_http_fetch_empty_url() {
    assert!(http_fetch("").is_err());
}

// @g8.capability(name = "auth-session-check", description = "Validate an inbound session cookie against the auth store", substrate = "auth", consumes = ["substrate::SessionCookie"], produces = ["substrate::AuthResult"], status = "in_flight")
/// Checks whether a session cookie is valid.
pub fn check_session(cookie: &str) -> bool {
    !cookie.is_empty()
}

// @g8.convergence_test(for_capability = "auth-session-check", scenario = "empty cookie returns false")
#[test]
fn test_auth_session_check_empty() {
    assert!(!check_session(""));
}
