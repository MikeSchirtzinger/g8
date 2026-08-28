//! Typed ID newtypes with deterministic, content-derived values.
//!
//! # Derivation contract (Q4 ruling, 2026-07-02 — decision-gate option 1)
//!
//! IDs are NOT random. Each ID is derived as `SHA-256(kind ␟ part₁ ␟ part₂ …)`
//! (`␟` = ASCII 0x1F unit separator, preventing concatenation ambiguity),
//! mapped onto 21 characters of the URL-safe alphabet (`A-Za-z0-9_-`) the
//! previous nanoid scheme used — same length, same alphabet, zero randomness.
//! The `kind` tag is the newtype's own name, so two ID types derived from
//! identical parts never collide.
//!
//! Determinism property: the same canonical identity always derives the same
//! ID, across processes, machines, and runs — which is what makes re-scanning
//! an unchanged tree byte-stable and keeps `getrandom`/`rand` out of the
//! deterministic zone's dependency graph entirely (OBL-D11-02).
//!
//! Pre-Q4 stores hold 21-char nanoid values in the same alphabet; those
//! remain valid opaque identifiers via [`from_string`] — derivation applies
//! to NEW mints only, so no store migration is required for v0.1.
//!
//! Each ID type enforces non-emptiness at construction time via
//! [`from_string`]. Serialization/deserialization treats the ID as a plain
//! JSON string.
//!
//! [`from_string`]: SpaceId::from_string

use crate::error::G8Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Minimum length accepted by `from_string`. Derived IDs are 21 chars; we
/// accept anything non-empty for hand-crafted test fixtures and pre-Q4
/// nanoid-era stores.
const MIN_ID_LEN: usize = 1;

fn validate_id(s: &str, kind: &str) -> Result<(), G8Error> {
    if s.len() < MIN_ID_LEN {
        return Err(G8Error::InvalidId(format!("{kind} ID must not be empty")));
    }
    Ok(())
}

/// The 64-symbol URL-safe alphabet shared with the retired nanoid scheme.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/// Derive a 21-character deterministic ID from a kind tag plus canonical
/// identity parts. Pure function of its inputs — no clock, no RNG.
fn derive_raw(kind: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    digest
        .iter()
        .take(21)
        .map(|b| ALPHABET[(b & 63) as usize] as char)
        .collect()
}

// ── Macro to define all ID newtypes with the same impl block ────────────────

macro_rules! define_id {
    ($name:ident, $kind:literal) => {
        /// Typed ID newtype for
        #[doc = $kind]
        /// entities.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Derive the deterministic ID for the entity whose canonical
            /// identity is `parts` (see the module docs for the derivation
            /// contract). Same parts in, same ID out — always.
            pub fn derive(parts: &[&str]) -> Self {
                Self(derive_raw($kind, parts))
            }

            /// Construct from an existing string, validating it is non-empty.
            pub fn from_string(s: String) -> Result<Self, G8Error> {
                validate_id(&s, $kind)?;
                Ok(Self(s))
            }

            /// TEST-ONLY convenience: a process-locally unique ID from a
            /// deterministic atomic counter (no randomness, no clock). IDs
            /// are only unique within one process run — never use this in
            /// production paths, which must pass real canonical identity to
            /// [`Self::derive`].
            #[doc(hidden)]
            pub fn sequential_for_tests() -> Self {
                static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Self(derive_raw($kind, &["test-seq", &n.to_string()]))
            }

            /// Borrow the underlying string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id!(SpaceId, "SpaceId");
define_id!(ProjectId, "ProjectId");
define_id!(CapabilityId, "CapabilityId");
define_id!(IntentId, "IntentId");
define_id!(PlanId, "PlanId");
define_id!(DecisionId, "DecisionId");
define_id!(AnnotationId, "AnnotationId");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic() {
        let a = SpaceId::derive(&["/repo/root", "default"]);
        let b = SpaceId::derive(&["/repo/root", "default"]);
        assert_eq!(a, b);
        assert_eq!(a.as_str().len(), 21);
    }

    #[test]
    fn derive_distinguishes_parts() {
        let a = SpaceId::derive(&["/repo/root", "default"]);
        let b = SpaceId::derive(&["/repo/root", "other"]);
        assert_ne!(a, b);
    }

    #[test]
    fn derive_has_no_concatenation_ambiguity() {
        // ("ab", "c") must not collide with ("a", "bc").
        let a = PlanId::derive(&["ab", "c"]);
        let b = PlanId::derive(&["a", "bc"]);
        assert_ne!(a, b);
    }

    #[test]
    fn derive_namespaces_by_kind() {
        // Same parts, different ID types — never the same value.
        let space = SpaceId::derive(&["x"]);
        let project = ProjectId::derive(&["x"]);
        assert_ne!(space.as_str(), project.as_str());
    }

    #[test]
    fn derive_uses_url_safe_alphabet_only() {
        let id = CapabilityId::derive(&["weird content \u{1f}\n\t", ""]);
        assert!(id
            .as_str()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn from_string_roundtrips() {
        let s = "hello-world-test-123".to_string();
        let id = PlanId::from_string(s.clone()).unwrap();
        assert_eq!(id.as_str(), s);
    }

    #[test]
    fn from_string_accepts_nanoid_era_ids() {
        // Pre-Q4 stores hold 21-char nanoid values; they must stay valid.
        let id = ProjectId::from_string("2PISSd60W-Ja1mR18K4Rb".into()).unwrap();
        assert_eq!(id.as_str().len(), 21);
    }

    #[test]
    fn from_string_rejects_empty() {
        assert!(ProjectId::from_string(String::new()).is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let id = CapabilityId::derive(&["proj", "http-fetch"]);
        let json = serde_json::to_string(&id).unwrap();
        let back: CapabilityId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn display_impl() {
        let id = DecisionId::from_string("abc".into()).unwrap();
        assert_eq!(format!("{id}"), "abc");
    }
}
