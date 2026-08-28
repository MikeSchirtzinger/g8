// NO-detect fixture: Rust file with no G8 annotations.

// This is a normal comment that should not be detected.
// Copyright 2026 example team

/// Documentation comment — not an G8 annotation.
pub fn regular_function(x: u32) -> u32 {
    x + 1
}

// TODO: add error handling
pub fn another_function() {}

// @some_other_annotation(foo = "bar") — not an g8 annotation
pub fn with_other_annotation() {}
