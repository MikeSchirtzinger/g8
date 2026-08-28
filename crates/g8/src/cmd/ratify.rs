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

    let lock_path = project_root.join(G8_LOCK_RELATIVE);
    let lock_text = match std::fs::read_to_string(&lock_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RatificationStatus::Unratified {
                detail: format!(
                    "{G8_LOCK_RELATIVE} is missing; run `g8 ratify` after reviewing the spec"
                ),
            };
        }
        Err(error) => {
            return RatificationStatus::Unratified {
                detail: format!("cannot read {G8_LOCK_RELATIVE}: {error}"),
            };
        }
    };
    let lock: G8Lock = match serde_json::from_str(&lock_text) {
        Ok(lock) => lock,
        Err(error) => {
            return RatificationStatus::Unratified {
                detail: format!("{G8_LOCK_RELATIVE} is invalid JSON: {error}"),
            };
        }
    };
    if lock.spec_path != OBLIGATIONS_ARTIFACT_RELATIVE {
        return RatificationStatus::Unratified {
            detail: format!(
                "{G8_LOCK_RELATIVE} pins {}, expected {OBLIGATIONS_ARTIFACT_RELATIVE}",
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

pub fn project_root(ctx: &Ctx) -> Result<PathBuf> {
    ctx.g8_dir
        .parent()
        .map(Path::to_path_buf)
        .context("resolved .g8 directory has no project root")
}
