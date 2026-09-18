//! `g8 ratify` — pin the current obligations specification in `g8.lock`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use g8_obligations::hashing::sha256_file;
use serde::{Deserialize, Serialize};

use crate::cli::OutputMode;
use crate::ctx::Ctx;
use crate::render::{json, pretty};

use super::sidecar::write_pretty_json;

pub const OBLIGATIONS_ARTIFACT_RELATIVE: &str = "specs/obligations-v0.1.json";
pub const G8_LOCK_RELATIVE: &str = "g8.lock";

/// What `govern ratify` (0.1.0) wrote. Read where `g8.lock` is absent so a
/// project ratified before the rename stays ratified; never written.
pub const LEGACY_LOCK_RELATIVE: &str = "govern.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct G8Lock {
    #[serde(alias = "govern_version")]
    pub g8_version: String,
    pub spec_path: String,
    pub spec_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatificationStatus {
    NotApplicable,
    Ratified,
    Unratified { detail: String },
}

pub fn run(ctx: &Ctx) -> Result<i32> {
    let root = project_root(ctx)?;
    let spec_path = root.join(OBLIGATIONS_ARTIFACT_RELATIVE);
    if !spec_path.is_file() {
        anyhow::bail!(
            "obligations specification not found at {}",
            spec_path.display()
        );
    }

    let spec_sha256 =
        sha256_file(&spec_path).with_context(|| format!("hashing {}", spec_path.display()))?;
    let lock = G8Lock {
        g8_version: env!("CARGO_PKG_VERSION").to_string(),
        spec_path: OBLIGATIONS_ARTIFACT_RELATIVE.to_string(),
        spec_sha256: spec_sha256.clone(),
    };
    let lock_path = root.join(G8_LOCK_RELATIVE);
    write_pretty_json(&lock_path, &lock)?;

    let message = format!(
        "Ratified {OBLIGATIONS_ARTIFACT_RELATIVE} at {spec_sha256}; wrote {G8_LOCK_RELATIVE}."
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

pub fn inspect(project_root: &Path) -> RatificationStatus {
    let spec_path = project_root.join(OBLIGATIONS_ARTIFACT_RELATIVE);
    if !spec_path.is_file() {
        return RatificationStatus::NotApplicable;
    }

    let (lock_name, lock_text) = match read_lock(project_root) {
        Ok(found) => found,
        Err(detail) => return RatificationStatus::Unratified { detail },
    };
    let lock: G8Lock = match serde_json::from_str(&lock_text) {
        Ok(lock) => lock,
        Err(error) => {
            return RatificationStatus::Unratified {
                detail: format!("{lock_name} is invalid JSON: {error}"),
            };
        }
    };
    if lock.spec_path != OBLIGATIONS_ARTIFACT_RELATIVE {
        return RatificationStatus::Unratified {
            detail: format!(
                "{lock_name} pins {}, expected {OBLIGATIONS_ARTIFACT_RELATIVE}",
                lock.spec_path
            ),
        };
    }

    let current = match sha256_file(&spec_path) {
        Ok(hash) => hash,
        Err(error) => {
            return RatificationStatus::Unratified {
                detail: format!("cannot hash {OBLIGATIONS_ARTIFACT_RELATIVE}: {error}"),
            };
        }
    };
    if current == lock.spec_sha256 {
        RatificationStatus::Ratified
    } else {
        RatificationStatus::Unratified {
            detail: format!(
                "{OBLIGATIONS_ARTIFACT_RELATIVE} changed after ratification (locked {}, current {}); review and run `g8 ratify`",
                lock.spec_sha256, current
            ),
        }
    }
}

/// Read `g8.lock`, or `govern.lock` when only the legacy file exists.
/// Returns the name that was read alongside its text so messages point at
/// the real file.
fn read_lock(project_root: &Path) -> std::result::Result<(&'static str, String), String> {
    let mut last_error = None;
    for name in [G8_LOCK_RELATIVE, LEGACY_LOCK_RELATIVE] {
        match std::fs::read_to_string(project_root.join(name)) {
            Ok(text) => return Ok((name, text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(format!("cannot read {name}: {error}")),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        format!("{G8_LOCK_RELATIVE} is missing; run `g8 ratify` after reviewing the spec")
    }))
}

pub fn project_root(ctx: &Ctx) -> Result<PathBuf> {
    ctx.g8_dir
        .parent()
        .map(Path::to_path_buf)
        .context("resolved .g8 directory has no project root")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_spec() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let spec = dir.path().join(OBLIGATIONS_ARTIFACT_RELATIVE);
        std::fs::create_dir_all(spec.parent().unwrap()).expect("specs dir");
        std::fs::write(&spec, b"{\"obligations\": []}").expect("write spec");
        let hash = sha256_file(&spec).expect("hash spec");
        (dir, hash)
    }

    fn lock_json(version_key: &str, version: &str, hash: &str) -> String {
        format!(
            "{{\"{version_key}\":\"{version}\",\"spec_path\":\"{OBLIGATIONS_ARTIFACT_RELATIVE}\",\"spec_sha256\":\"{hash}\"}}"
        )
    }

    #[test]
    fn inspect_reads_legacy_govern_lock() {
        let (dir, hash) = project_with_spec();
        std::fs::write(
            dir.path().join(LEGACY_LOCK_RELATIVE),
            lock_json("govern_version", "0.1.0", &hash),
        )
        .expect("write govern.lock");
        assert_eq!(inspect(dir.path()), RatificationStatus::Ratified);
    }

    #[test]
    fn inspect_prefers_g8_lock_over_legacy() {
        let (dir, hash) = project_with_spec();
        std::fs::write(
            dir.path().join(LEGACY_LOCK_RELATIVE),
            lock_json("govern_version", "0.1.0", &hash),
        )
        .expect("write govern.lock");
        std::fs::write(
            dir.path().join(G8_LOCK_RELATIVE),
            lock_json("g8_version", "0.1.1", "sha256:stale"),
        )
        .expect("write g8.lock");
        match inspect(dir.path()) {
            RatificationStatus::Unratified { detail } => {
                assert!(detail.contains("changed after ratification"), "{detail}")
            }
            other => panic!("expected the g8.lock verdict, got {other:?}"),
        }
    }

    #[test]
    fn inspect_without_any_lock_is_unratified() {
        let (dir, _) = project_with_spec();
        match inspect(dir.path()) {
            RatificationStatus::Unratified { detail } => {
                assert!(detail.contains("g8.lock is missing"), "{detail}")
            }
            other => panic!("expected unratified, got {other:?}"),
        }
    }
}
