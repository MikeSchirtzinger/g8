//! HTTP ingestion service — example G8-annotated Rust crate.
//!
//! This crate demonstrates the `// @g8.capability(...)` and
//! `// @g8.convergence_test(...)` annotation grammar.  Run
//! `g8 scan examples/rust-service` from the workspace root to extract
//! all annotations and populate the local store.

use std::collections::HashMap;

// ── request-parse capability ────────────────────────────────────────────────

// @g8.capability(name = "request-parse", description = "Deserialize and validate inbound HTTP request bodies", substrate = "http-ingestion", consumes = ["substrate::HttpRequest"], produces = ["substrate::ParsedRequest"], status = "landed")
/// Parses a raw HTTP request body into a typed `ParsedRequest`.
///
/// Returns `Err` with a structured `ParseError` if the body is malformed or
/// exceeds the maximum allowed size.
pub fn parse_request(body: &[u8]) -> Result<ParsedRequest, ParseError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ParseError::BodyTooLarge {
            size: body.len(),
            limit: MAX_BODY_BYTES,
        });
    }
    let map: HashMap<String, serde_json::Value> =
        serde_json::from_slice(body).map_err(|e| ParseError::Json(e.to_string()))?;
    Ok(ParsedRequest { fields: map })
}

// @g8.convergence_test(for_capability = "request-parse", scenario = "oversized body rejected with BodyTooLarge")
#[test]
fn test_request_parse_body_too_large() {
    let oversized = vec![b'x'; MAX_BODY_BYTES + 1];
    assert!(matches!(
        parse_request(&oversized),
        Err(ParseError::BodyTooLarge { .. })
    ));
}

// ── auth-token-validate capability ─────────────────────────────────────────

// @g8.capability(name = "auth-token-validate", description = "Verify a Bearer token against the token store", substrate = "auth", consumes = ["substrate::BearerToken"], produces = ["substrate::ClaimsSet"], status = "in_flight", stub = true, since = "2026-05-26")
/// Validates a bearer token and returns the associated claims.
///
/// Stub: full validation is in-flight.  Returns `Ok(ClaimsSet::anonymous())`
/// until the token store integration lands.
pub fn validate_token(_token: &str) -> Result<ClaimsSet, AuthError> {
    // TODO: call token store
    Ok(ClaimsSet::anonymous())
}

// @g8.convergence_test(for_capability = "auth-token-validate", scenario = "empty token returns anonymous claims (stub behaviour)")
#[test]
fn test_auth_token_validate_stub_returns_anonymous() {
    let claims = validate_token("").unwrap();
    assert!(claims.is_anonymous());
}

// ── request-route capability ────────────────────────────────────────────────

// @g8.capability(name = "request-route", description = "Route a parsed request to the correct downstream handler", substrate = "http-ingestion", consumes = ["substrate::ParsedRequest", "substrate::ClaimsSet"], produces = ["substrate::HandlerResponse"], status = "in_flight")
/// Routes a validated, parsed request to the appropriate handler function.
pub fn route_request(req: ParsedRequest, claims: ClaimsSet) -> HandlerResponse {
    let _ = claims; // authorization check in-flight
    if req.fields.contains_key("ping") {
        HandlerResponse::Pong
    } else {
        HandlerResponse::Accepted { id: "stub-id".into() }
    }
}

// @g8.convergence_test(for_capability = "request-route", scenario = "ping payload returns Pong")
#[test]
fn test_request_route_ping() {
    let mut fields = HashMap::new();
    fields.insert("ping".into(), serde_json::Value::Bool(true));
    let req = ParsedRequest { fields };
    let claims = ClaimsSet::anonymous();
    assert!(matches!(route_request(req, claims), HandlerResponse::Pong));
}

// ── metrics-emit capability ─────────────────────────────────────────────────

// @g8.capability(name = "metrics-emit", description = "Emit request-level metrics to the telemetry substrate", substrate = "observability", consumes = ["substrate::HandlerResponse"], produces = ["substrate::MetricEvent"], status = "proposed")
/// Emits a metric event after a request completes.
pub fn emit_metric(response: &HandlerResponse, latency_ms: u64) -> MetricEvent {
    let status = match response {
        HandlerResponse::Pong => "pong",
        HandlerResponse::Accepted { .. } => "accepted",
    };
    MetricEvent {
        label: format!("http.request.{status}"),
        value: latency_ms,
    }
}

// @g8.convergence_test(for_capability = "metrics-emit", scenario = "accepted response produces correct label")
#[test]
fn test_metrics_emit_label() {
    let resp = HandlerResponse::Accepted { id: "x".into() };
    let event = emit_metric(&resp, 12);
    assert_eq!(event.label, "http.request.accepted");
    assert_eq!(event.value, 12);
}

// ── Data types ──────────────────────────────────────────────────────────────

const MAX_BODY_BYTES: usize = 1_048_576; // 1 MiB

#[derive(Debug)]
pub struct ParsedRequest {
    pub fields: HashMap<String, serde_json::Value>,
}

#[derive(Debug)]
pub struct ClaimsSet {
    pub subject: Option<String>,
}

impl ClaimsSet {
    pub fn anonymous() -> Self {
        Self { subject: None }
    }
    pub fn is_anonymous(&self) -> bool {
        self.subject.is_none()
    }
}

#[derive(Debug)]
pub enum HandlerResponse {
    Pong,
    Accepted { id: String },
}

#[derive(Debug)]
pub struct MetricEvent {
    pub label: String,
    pub value: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("body too large: {size} bytes (limit: {limit})")]
    BodyTooLarge { size: usize, limit: usize },
    #[error("JSON parse error: {0}")]
    Json(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid token")]
    InvalidToken,
}
