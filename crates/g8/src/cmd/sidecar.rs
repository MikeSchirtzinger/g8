//! Shared atomic JSON sidecar writer used by `attest` and `ratify`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

pub(crate) fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating sidecar directory {}", parent.display()))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("{} has no UTF-8 file name", path.display()))?;
    let temporary = path.with_file_name(format!(".{file_name}.tmp"));
    let mut bytes = serde_json::to_vec_pretty(value).context("serializing JSON sidecar")?;
    bytes.push(b'\n');

    std::fs::write(&temporary, bytes)
        .with_context(|| format!("writing temporary sidecar {}", temporary.display()))?;
    if let Err(source) = std::fs::rename(&temporary, path) {
        let _cleanup = std::fs::remove_file(&temporary);
        return Err(source).with_context(|| format!("installing sidecar {}", path.display()));
    }
    Ok(())
}
