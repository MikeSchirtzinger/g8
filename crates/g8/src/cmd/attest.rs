//! `g8 attest` — write hash-pinned evidence into the attestation sidecar.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use g8_obligations::hashing::{normalize_repo_paths, sha256_repo_files};
use g8_obligations::{
    attestation_sidecar_path, load_artifact, load_attestation_sidecar, AttestationClaim,
    AttestationRecord, AttestationSidecar, ObligationChecker,
};

use crate::cli::{AttestArgs, OutputMode};
use crate::ctx::Ctx;
use crate::render::{json, pretty};

use super::ratify::{project_root, OBLIGATIONS_ARTIFACT_RELATIVE};
use super::sidecar::write_pretty_json;

pub fn run(ctx: &Ctx, args: &AttestArgs) -> Result<i32> {
    let root = project_root(ctx)?;
    let artifact_path = root.join(OBLIGATIONS_ARTIFACT_RELATIVE);
    let artifact = load_artifact(&artifact_path)
        .with_context(|| format!("loading {}", artifact_path.display()))?;
    let obligation = artifact
        .obligations
        .iter()
        .find(|obligation| obligation.id == args.obligation_id)
        .with_context(|| {
            format!(
                "obligation '{}' not found in {OBLIGATIONS_ARTIFACT_RELATIVE}",
                args.obligation_id
            )
        })?;

    let files = normalize_repo_paths(&root, &args.files).context("validating --files")?;
    let pinned_content_hash = sha256_repo_files(&root, &files).context("hashing attested files")?;
    let evidence_pointer = args.evidence.clone().unwrap_or_else(|| files.join(", "));
    if evidence_pointer.trim().is_empty() {
        anyhow::bail!("evidence pointer cannot be empty");
    }

    let pin_suffix = pinned_content_hash
        .strip_prefix("sha256:")
        .and_then(|hash| hash.get(..12))
        .unwrap_or("content-pin");
    let attestation_id = match obligation.checker.as_ref() {
        Some(ObligationChecker::Attestation { attestation_id }) => attestation_id.clone(),
        _ => format!("ATT-{}-{pin_suffix}", obligation.id),
    };
    let record = AttestationRecord {
        id: attestation_id,
        obligation_id: obligation.id.clone(),
        attested_at: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)),
        claim: AttestationClaim::Passed,
        evidence_pointer: Some(evidence_pointer.clone()),
        evidence_summary: None,
        files,
        pinned_content_hash: pinned_content_hash.clone(),
        expires_policy: "stale_on_hash_mismatch".to_string(),
        extra: BTreeMap::new(),
    };

    let mut sidecar = load_attestation_sidecar(&root)
        .context("loading existing attestation sidecar")?
        .unwrap_or_else(AttestationSidecar::default);
    // Preserve prior evidence records. Repeating an identical attestation is
    // idempotent, while a new content pin appends a new auditable record.
    sidecar
        .attestations
        .retain(|existing| existing.id != record.id);
    sidecar.attestations.push(record);

    let sidecar_path = attestation_sidecar_path(&root);
    write_pretty_json(&sidecar_path, &sidecar)?;

    let message = format!(
        "Attested {} at {}; evidence {}; wrote {}.",
        obligation.id,
        pinned_content_hash,
        evidence_pointer,
        g8_obligations::ATTESTATIONS_RELATIVE_PATH
    );
    match ctx.output {
        OutputMode::Json => json::render_ok(message),
        OutputMode::Pretty => {
            if !ctx.quiet {
                pretty::ok(&message, ctx.color);
            }
        }
    }
    Ok(0)
}
