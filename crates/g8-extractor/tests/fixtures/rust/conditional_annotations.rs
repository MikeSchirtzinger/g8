// PASS-detect fixture: Rust file with conditional (dev_only / cfg_attr) annotations.
// These should be detected but flagged as conditional = true.

#[cfg(test)]
// @g8.capability(name = "test-only-cap", status = "in_flight", dev_only = true)
pub fn test_helper() {}

// @g8.capability(name = "cfg-wrapped", status = "proposed", dev_only = true)
pub fn dev_only_function() {}
