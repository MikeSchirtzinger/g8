# Architecture

repoB is the background worker service.  It handles async jobs (email dispatch,
report generation, webhook delivery) and uses an outbound HTTP client for
third-party API calls.

# Boundaries

- Never expose synchronous endpoints; all entry points are job queue consumers.
- Outbound HTTP calls MUST go through the `http-fetch` capability.

# Stack

Rust, tokio, reqwest, lapin (AMQP)

# Owner

team: platform-workers
contact: #platform-eng
