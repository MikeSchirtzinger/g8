//! `g8 check --json` output contract — shared types.
//!
//! Relocated here from `crates/g8/src/render/json.rs` per
//! `specs/obligation-checker-contract.md` §8, as part of resolving the
//! `G8CheckContract` obligation backend's recursion hazard (that backend
//! validates the SHAPE of `g8 check --json`'s own output and must never do
//! so by shelling out to `g8 check` itself). Living at the root of the
//! crate DAG means `g8` (the real renderer) and `g8-obligations` (the
//! in-process validator) both see one canonical Rust type for the parts of
//! the contract that don't need a reverse dependency edge to name — see
//! [`CheckPayload`]'s own doc comment for why `obligations[]` itself is
//! deliberately NOT a field here.
//!
//! `g8-obligations`'s `G8CheckContract` executor does not actually consume
//! this module's types today — by design it validates a `serde_json::Value`
//! shape seam instead (see `g8-obligations/src/lib.rs`'s module docs), so
//! it never needed to wait on this relocation. This module is still the
//! single source of truth any future in-process consumer should prefer over
//! a second hand-written JSON shape.

use serde::{Deserialize, Serialize};

/// The stable `g8 check --json` contract (ARCHITECTURE.md §14 / SPEC
/// Decision 10), extended per `obligation-checker-contract.md` §5.
///
/// `enforcement_disabled` is omitted from the wire format entirely when
/// `None` — this preserves the pre-existing, locked behavior that an
/// enforcement-ON payload carries no `enforcement_disabled` key at all
/// (contract §5: "g8_version, errors, exit_code, enforcement_disabled are
/// unchanged in shape and unchanged in when they appear").
///
/// The `obligations[]` self-audit array (contract §5) is deliberately NOT a
/// field of this struct: it would need to be typed as
/// `Vec<g8_obligations::ObligationResult>`, and `g8-obligations` already
/// depends on `g8-core` — naming that type here would require the reverse
/// edge, a cycle. The renderer (`g8/src/render/json.rs::render_check`)
/// splices `obligations`/`obligations_note` onto this struct's serialized
/// form instead, the same pattern `render_plan_new_result` already uses for
/// `plan_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckPayload {
    #[serde(alias = "govern_version")]
    pub g8_version: String,
    pub errors: Vec<CheckErrorItem>,
    pub exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforcement_disabled: Option<bool>,
}

/// One entry in the `errors` array — a capability that fails the pairing
/// gate (no matching convergence test, an expired/missing stub, or a
/// duplicate name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckErrorItem {
    pub capability: String,
    pub file: String,
    pub line: u32,
    pub reason: PairingReason,
}

/// Closed reason vocabulary for [`CheckErrorItem::reason`], matching
/// ARCHITECTURE.md §14's stable list. `g8-core` is the root of the crate
/// DAG (SPEC §4) and cannot depend on `g8-store`, so this is deliberately
/// its own enum rather than a re-export of `g8_store::PairingErrorReason`
/// — `g8`'s renderer (`render::to_pairing_reason`) maps one onto the
/// other. `InvalidRoot` is intentionally absent: `StoreConnection::
/// pairing_check` never constructed it (dead code), deleted end-to-end per
/// `o-g8-defuse-and-wire-20260702.md` T5 acceptance criterion 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingReason {
    NoMatchingConvergenceTest,
    StubExpired,
    StubMissingSince,
    DuplicateCapabilityName,
}

impl PairingReason {
    /// The exact string this variant serializes to — a convenience accessor
    /// for plain-text (non-JSON) rendering so pretty-printers don't need
    /// their own parallel match arm over this enum.
    pub fn as_str(self) -> &'static str {
        match self {
            PairingReason::NoMatchingConvergenceTest => "no_matching_convergence_test",
            PairingReason::StubExpired => "stub_expired",
            PairingReason::StubMissingSince => "stub_missing_since",
            PairingReason::DuplicateCapabilityName => "duplicate_capability_name",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_payload_omits_enforcement_disabled_when_none() {
        let payload = CheckPayload {
            g8_version: "0.1.0".to_string(),
            errors: vec![],
            exit_code: 0,
            enforcement_disabled: None,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert!(value.get("enforcement_disabled").is_none());
    }

    #[test]
    fn check_payload_includes_enforcement_disabled_when_some() {
        let payload = CheckPayload {
            g8_version: "0.1.0".to_string(),
            errors: vec![],
            exit_code: 0,
            enforcement_disabled: Some(true),
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["enforcement_disabled"], serde_json::json!(true));
    }

    #[test]
    fn pairing_reason_serializes_to_locked_snake_case_strings() {
        assert_eq!(
            serde_json::to_value(PairingReason::NoMatchingConvergenceTest).unwrap(),
            serde_json::json!("no_matching_convergence_test")
        );
        assert_eq!(
            serde_json::to_value(PairingReason::DuplicateCapabilityName).unwrap(),
            serde_json::json!("duplicate_capability_name")
        );
    }

    #[test]
    fn pairing_reason_as_str_matches_serde_output() {
        for reason in [
            PairingReason::NoMatchingConvergenceTest,
            PairingReason::StubExpired,
            PairingReason::StubMissingSince,
            PairingReason::DuplicateCapabilityName,
        ] {
            let serialized = serde_json::to_value(reason).unwrap();
            assert_eq!(serialized, serde_json::json!(reason.as_str()));
        }
    }

    #[test]
    fn check_error_item_round_trips() {
        let item = CheckErrorItem {
            capability: "http-fetch".to_string(),
            file: "src/lib.rs".to_string(),
            line: 42,
            reason: PairingReason::StubExpired,
        };
        let value = serde_json::to_value(&item).unwrap();
        let back: CheckErrorItem = serde_json::from_value(value).unwrap();
        assert_eq!(back.capability, "http-fetch");
        assert_eq!(back.reason, PairingReason::StubExpired);
    }
}
