# Architecture

repoA is the primary API gateway service.  It owns TLS termination, request
routing, and the outbound HTTP client pool used for upstream service calls.

# Boundaries

- Never hold auth tokens in memory longer than one request lifecycle.
- Never make outbound HTTP calls outside the `http-fetch` capability.

# Stack

Rust, tokio, reqwest, rustls

# Owner

team: platform-gateway
contact: #platform-eng
