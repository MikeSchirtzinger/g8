//! `specs/obligations-v0.1.json` deserialization — contract §6.
//!
//! Models the fields this crate needs (`id`, `checker`, `signal`). All other
//! fields on an obligation object (`descends_from`, `rule`, `scope`,
//! `convergence_test`, `also_stated_by`) are T4's domain, stay untouched per
//! contract §6's binding rule for T4, and are simply ignored here — no
//! `deny_unknown_fields`, so the artifact can keep evolving those fields
//! without breaking this deserializer.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::backend::CheckerBackend;

/// One obligation's compiled, executable checker — mutually exclusive with
/// the other two modes via the `mode` tag (contract §6).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ObligationChecker {
    /// §2's mapping — all 27, today.
    Typed { checks: Vec<CheckerBackend> },
    /// Unused by the v1 mapping (0/27) but fully specified — the fallback
    /// the moment an obligation proves genuinely infeasible to check
    /// mechanically. See contract §7 and [`crate::attestation`].
    Attestation { attestation_id: String },
    /// Unused by the v1 mapping (0/27).
    ProseOnly,
}

/// An obligation's pre-existing `signal` block (predates this contract —
/// original artifact field, untouched by T4's `checker` addition).
///
/// Exposed here (T3b) so `g8`'s check-integration no longer needs a
/// second, tolerant raw-JSON re-parse of the artifact just to read
/// `signal.advisory` for exit-code wiring (contract §5.1: `advisory: true`
/// obligations never affect exit code) — that was T5's escalation #1.
/// Shape verified against all 27 real obligations, not assumed: every one
/// carries exactly these four fields, `conflict_kind` the only ever-`null`
/// one.
///
/// Every field is individually `#[serde(default)]`: T5's own test fixtures
/// (`crates/g8/tests/obligations_check.rs`, predating this exposure)
/// use a deliberately minimal `{"advisory": false}` shape for
/// wiring-logic-only tests — real production data always has all four
/// fields, but this crate should not hard-fail a smaller, legitimate test
/// fixture just because it only populated the one field its test actually
/// exercises. Consistent with `Obligation.checker`/`Obligation.signal`
/// themselves: never require a field this crate can gracefully do without.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Signal {
    #[serde(default)]
    pub gate: String,
    #[serde(default)]
    pub conflict_kind: Option<String>,
    #[serde(default)]
    pub wiring: String,
    /// `true` ⇒ this obligation's `Failed`/`Error` status never affects
    /// `g8 check`'s exit code (contract §5.1). Currently 2/27 (D6-02,
    /// G1-01). Defaults to `false` if absent — the conservative direction
    /// (an obligation missing this field is treated as non-advisory, so its
    /// failures still affect the exit code, never silently suppressed).
    #[serde(default)]
    pub advisory: bool,
}

/// One obligation object from `specs/obligations-v0.1.json`, narrowed to the
/// fields `run_obligations` actually consumes.
#[derive(Debug, Clone, Deserialize)]
pub struct Obligation {
    pub id: String,
    /// Absent entirely (older artifact revisions, or a future obligation
    /// added without a `checker` field yet) maps to `status = Unknown`, not
    /// a deserialization error — per contract §3.1's explicit "must produce
    /// it gracefully — never panic — for any future obligation added
    /// without a `checker` field."
    #[serde(default)]
    pub checker: Option<ObligationChecker>,
    /// `#[serde(default)]` (not required) for the same reason as `checker`:
    /// a future obligation or hand-built test fixture might omit it — every
    /// one of the real 27 has it, but this crate never hard-requires a field
    /// it can gracefully do without.
    #[serde(default)]
    pub signal: Option<Signal>,
}

/// Deserialized `specs/obligations-v0.1.json`.
///
/// `meta`/`conflicts`/`open_questions` are kept as opaque `serde_json::Value`
/// — this crate has no reason to interpret them, and modeling them strongly
/// would create exactly the kind of accidental coupling to T4's document
/// shape this crate should not have.
#[derive(Debug, Clone, Deserialize)]
pub struct ObligationArtifact {
    #[serde(default)]
    pub meta: serde_json::Value,
    pub obligations: Vec<Obligation>,
    #[serde(default)]
    pub conflicts: serde_json::Value,
    #[serde(default)]
    pub open_questions: serde_json::Value,
}

/// Errors from loading/parsing the obligations artifact.
#[derive(Debug, thiserror::Error)]
pub enum ObligationsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse obligations artifact JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Load and deserialize `specs/obligations-v0.1.json` (or any path in the
/// same shape).
pub fn load_artifact(path: &Path) -> Result<ObligationArtifact, ObligationsError> {
    let text = fs::read_to_string(path)?;
    let artifact: ObligationArtifact = serde_json::from_str(&text)?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contract §6's own worked example must deserialize into our schema
    /// types exactly — pre-completion checklist item #5. One deliberate
    /// deviation from the contract's literal text: `signal` is given
    /// OBL-P9-01's REAL values (verified against the actual artifact) in
    /// place of the contract's own `{"...": "... UNTOUCHED ..."}`
    /// placeholder — that placeholder was never meant literally (same
    /// convention used for `descends_from`/`rule`/`scope` here, still
    /// placeholders, still untyped by this crate), but `signal` became a
    /// real, typed field in T3b and needs a real shape to round-trip.
    const CONTRACT_SECTION_6_EXAMPLE: &str = r#"
    {
      "id": "OBL-P9-01",
      "descends_from": { "...": "... UNTOUCHED ..." },
      "also_stated_by": ["..."],
      "rule": { "...": "... UNTOUCHED ..." },
      "scope": { "...": "... UNTOUCHED ..." },
      "convergence_test": {
        "id": "conv-p9-01",
        "check": "cargo metadata --format-version1 | jq '.packages[].dependencies[].name'; assert 'surrealdb' and 'graph-tool' are absent...",
        "fails_when": "... UNTOUCHED: this is documentation, not the executable spec ..."
      },
      "checker": {
        "mode": "typed",
        "checks": [
          {
            "backend": "cargo_metadata_no_dep",
            "args": {
              "denied": [{"kind": "exact", "value": "surrealdb"}, {"kind": "exact", "value": "graph-tool"}],
              "scope_crates": [],
              "build_config": "default"
            }
          }
        ]
      },
      "signal": {
        "gate": "g8-conflict",
        "conflict_kind": "GovernanceViolation",
        "wiring": "compiled_intent_extension",
        "advisory": false
      }
    }
    "#;

    #[test]
    fn contract_section_6_example_round_trips_exactly() {
        let obligation: Obligation =
            serde_json::from_str(CONTRACT_SECTION_6_EXAMPLE).expect("must deserialize");
        assert_eq!(obligation.id, "OBL-P9-01");
        let checker = obligation.checker.expect("checker present");
        match checker {
            ObligationChecker::Typed { checks } => {
                assert_eq!(checks.len(), 1);
                assert_eq!(checks[0].name(), "cargo_metadata_no_dep");
                let CheckerBackend::CargoMetadataNoDep(args) = &checks[0] else {
                    panic!("expected CargoMetadataNoDep, got {:?}", checks[0]);
                };
                assert_eq!(args.denied.len(), 2);
                assert!(args.scope_crates.is_empty());
                assert!(matches!(
                    args.build_config,
                    crate::backend::BuildConfig::Default
                ));
            }
            other => panic!("expected Typed, got {other:?}"),
        }
        let signal = obligation.signal.expect("signal present");
        assert_eq!(signal.gate, "g8-conflict");
        assert!(!signal.advisory);
    }

    #[test]
    fn missing_checker_field_deserializes_to_none() {
        let json = r#"{"id": "OBL-X-01"}"#;
        let obligation: Obligation = serde_json::from_str(json).expect("must deserialize");
        assert!(obligation.checker.is_none());
        assert!(obligation.signal.is_none());
    }

    #[test]
    fn signal_deserializes_with_real_shape() {
        let json = r#"{
            "id": "OBL-P9-01",
            "signal": {
                "gate": "g8-conflict",
                "conflict_kind": "GovernanceViolation",
                "wiring": "compiled_intent_extension",
                "advisory": false
            }
        }"#;
        let obligation: Obligation = serde_json::from_str(json).expect("must deserialize");
        let signal = obligation.signal.expect("signal present");
        assert_eq!(signal.gate, "g8-conflict");
        assert_eq!(signal.conflict_kind.as_deref(), Some("GovernanceViolation"));
        assert_eq!(signal.wiring, "compiled_intent_extension");
        assert!(!signal.advisory);
    }

    #[test]
    fn signal_conflict_kind_null_deserializes_to_none() {
        let json = r#"{
            "id": "OBL-G1-01",
            "signal": {
                "gate": "codegraph_vocabulary_drift",
                "conflict_kind": null,
                "wiring": "compiled_intent_extension",
                "advisory": true
            }
        }"#;
        let obligation: Obligation = serde_json::from_str(json).expect("must deserialize");
        let signal = obligation.signal.expect("signal present");
        assert_eq!(signal.conflict_kind, None);
        assert!(signal.advisory);
    }

    #[test]
    fn signal_tolerates_a_minimal_advisory_only_fixture() {
        // Exact shape `crates/g8/tests/obligations_check.rs` uses for
        // its own wiring-logic-only test fixtures (predates this exposure) —
        // this crate must not hard-require gate/conflict_kind/wiring just
        // because a real production artifact always happens to have them.
        let json = r#"{"id": "OBL-X-01", "signal": {"advisory": true}}"#;
        let obligation: Obligation = serde_json::from_str(json).expect("must deserialize");
        let signal = obligation.signal.expect("signal present");
        assert!(signal.advisory);
        assert_eq!(signal.gate, "");
        assert_eq!(signal.conflict_kind, None);
    }

    #[test]
    fn attestation_mode_round_trips() {
        let json =
            r#"{"id": "OBL-X-01", "checker": {"mode": "attestation", "attestation_id": "ATT-1"}}"#;
        let obligation: Obligation = serde_json::from_str(json).expect("must deserialize");
        match obligation.checker.unwrap() {
            ObligationChecker::Attestation { attestation_id } => {
                assert_eq!(attestation_id, "ATT-1");
            }
            other => panic!("expected Attestation, got {other:?}"),
        }
    }

    #[test]
    fn prose_only_mode_round_trips() {
        let json = r#"{"id": "OBL-X-01", "checker": {"mode": "prose_only"}}"#;
        let obligation: Obligation = serde_json::from_str(json).expect("must deserialize");
        assert!(matches!(
            obligation.checker.unwrap(),
            ObligationChecker::ProseOnly
        ));
    }

    #[test]
    fn full_artifact_wrapper_deserializes_ignoring_unknown_sibling_fields() {
        let json = r#"{
            "meta": {"artifact": "g8-obligations"},
            "obligations": [
                {"id": "OBL-A-01", "extra_field_t4_might_add": true},
                {"id": "OBL-B-01", "checker": {"mode": "prose_only"}}
            ],
            "conflicts": [],
            "open_questions": []
        }"#;
        let artifact: ObligationArtifact = serde_json::from_str(json).expect("must deserialize");
        assert_eq!(artifact.obligations.len(), 2);
        assert_eq!(artifact.obligations[0].id, "OBL-A-01");
        assert!(artifact.obligations[0].checker.is_none());
    }

    #[test]
    fn load_artifact_reads_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("obligations.json");
        std::fs::write(
            &path,
            r#"{"obligations": [{"id": "OBL-A-01", "checker": {"mode": "prose_only"}}]}"#,
        )
        .unwrap();
        let artifact = load_artifact(&path).expect("load ok");
        assert_eq!(artifact.obligations.len(), 1);
    }

    #[test]
    fn load_artifact_missing_file_is_error_not_panic() {
        let result = load_artifact(Path::new("/nonexistent/path/obligations.json"));
        assert!(result.is_err());
    }

    #[test]
    fn load_artifact_malformed_json_is_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{ not json").unwrap();
        let result = load_artifact(&path);
        assert!(result.is_err());
    }

    /// The decisive T3↔T4 integration proof (added after the
    /// T2b/T4b amendments landed): load the REAL `specs/obligations-v0.1.json`
    /// — the wire-format ground truth, not this crate's own assumptions —
    /// and deserialize every one of its 27 obligations' `checker` fields
    /// with zero errors. `CheckerBackend` is a closed enum (contract §0): if
    /// a `"backend"` tag value here didn't match one of the 10 known
    /// variants, `serde_json::from_str` would itself fail — so successful
    /// deserialization of all 27 checkers already proves every check
    /// resolves to a known variant, not just that JSON parsing succeeded.
    #[test]
    fn real_obligations_artifact_deserializes_all_27_with_zero_errors() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("specs/obligations-v0.1.json");

        let artifact = load_artifact(&path).unwrap_or_else(|e| {
            panic!(
                "real artifact at {} failed to deserialize: {e}",
                path.display()
            )
        });

        assert_eq!(
            artifact.obligations.len(),
            27,
            "expected all 27 obligations, got {}",
            artifact.obligations.len()
        );

        let mut typed_count = 0usize;
        let mut total_checks = 0usize;
        for o in &artifact.obligations {
            match &o.checker {
                Some(ObligationChecker::Typed { checks }) => {
                    assert!(
                        !checks.is_empty(),
                        "{}: checker.mode = typed but checks[] is empty",
                        o.id
                    );
                    typed_count += 1;
                    total_checks += checks.len();
                }
                other => panic!(
                    "{}: expected checker.mode = typed (contract §2's own tally: 27 typed / 0 \
                     attestation / 0 prose_only), got {other:?}",
                    o.id
                ),
            }
        }
        assert_eq!(typed_count, 27, "expected all 27 obligations to be typed");
        // D6-01/P6-01/D1-01/D11-01/N4-01/N8-01 etc. carry >1 check each
        // (contract §2's own ×2/×3 rows) — sanity floor, not a precise count.
        assert!(
            total_checks >= 27,
            "expected at least one check per obligation, got {total_checks} total checks across 27 obligations"
        );

        // `signal` (T3b): every real obligation carries one, and exactly 2
        // (D6-02, G1-01) are advisory — verified independently against the
        // real file before writing this assertion, not assumed.
        let mut advisory_count = 0usize;
        for o in &artifact.obligations {
            let signal = o
                .signal
                .as_ref()
                .unwrap_or_else(|| panic!("{}: expected a signal block, got none", o.id));
            assert!(!signal.gate.is_empty(), "{}: signal.gate is empty", o.id);
            assert!(
                !signal.wiring.is_empty(),
                "{}: signal.wiring is empty",
                o.id
            );
            if signal.advisory {
                advisory_count += 1;
            }
        }
        assert_eq!(
            advisory_count, 2,
            "expected exactly 2 advisory obligations (D6-02, G1-01), got {advisory_count}"
        );
    }
}
