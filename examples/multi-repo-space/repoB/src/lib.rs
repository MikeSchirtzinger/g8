//! repoB — background worker service.
//!
//! NOTE: this crate deliberately declares an `http-fetch` capability with the
//! same name as repoA.  Running `g8 merge --from repoB --into repoA` will
//! detect this as a `CapabilityNameCollision` (Error severity).
//!
//! Resolution: run `g8 link --canonical "repoA::http-fetch" --alias "repoB::http-fetch"`
//! to declare them equivalent, or rename one of them.

// @g8.capability(name = "http-fetch", description = "Outbound HTTP client for third-party webhook delivery", substrate = "http-layer", consumes = ["substrate::WebhookPayload"], produces = ["substrate::DeliveryReceipt"], status = "landed")
/// Deliver a webhook payload to a third-party URL.
pub fn http_fetch(url: &str, payload: &[u8]) -> Result<u16, String> {
    if url.is_empty() {
        return Err("empty URL".into());
    }
    let _ = payload;
    Ok(200)
}

// @g8.convergence_test(for_capability = "http-fetch", scenario = "empty URL returns error")
#[test]
fn test_http_fetch_empty_url() {
    assert!(http_fetch("", b"{}").is_err());
}

// @g8.capability(name = "job-dequeue", description = "Dequeue and deserialize a job from the AMQP queue", substrate = "job-queue", consumes = ["substrate::AmqpMessage"], produces = ["substrate::Job"], status = "landed")
/// Dequeue one job from the worker queue.
pub fn dequeue_job(raw: &[u8]) -> Result<Job, String> {
    let s = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
    Ok(Job { kind: s.to_string() })
}

// @g8.convergence_test(for_capability = "job-dequeue", scenario = "valid UTF-8 payload returns Job")
#[test]
fn test_dequeue_job_valid_utf8() {
    let job = dequeue_job(b"email-dispatch").unwrap();
    assert_eq!(job.kind, "email-dispatch");
}

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Job {
    pub kind: String,
}
