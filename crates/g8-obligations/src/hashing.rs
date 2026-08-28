//! Shared SHA-256 helpers for ratification locks and evidence attestations.
//!
//! Both writers and readers use these functions so the pin format and the
//! multi-file framing cannot drift apart. Paths stored in sidecars are always
//! canonical, repository-relative UTF-8 paths.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const SHA256_PREFIX: &str = "sha256:";

#[derive(Debug, thiserror::Error)]
pub enum HashingError {
    #[error("cannot hash an empty file set")]
    EmptyFileSet,
    #[error("I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("path {path} is outside workspace root {workspace_root}")]
    OutsideWorkspace {
        path: PathBuf,
        workspace_root: PathBuf,
    },
    #[error("path {0} is not valid UTF-8")]
    NonUtf8Path(PathBuf),
}

/// Hash one file's exact bytes and return the locked `sha256:<hex>` form.
pub fn sha256_file(path: &Path) -> Result<String, HashingError> {
    let mut file = File::open(path).map_err(|source| HashingError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let read = file.read(&mut buffer).map_err(|source| HashingError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{SHA256_PREFIX}{:x}", hasher.finalize()))
}

/// Resolve input files beneath `workspace_root`, returning sorted, deduplicated
/// repository-relative paths suitable for a JSON sidecar.
pub fn normalize_repo_paths(
    workspace_root: &Path,
    input_paths: &[PathBuf],
) -> Result<Vec<String>, HashingError> {
    if input_paths.is_empty() {
        return Err(HashingError::EmptyFileSet);
    }

    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|source| HashingError::Io {
            path: workspace_root.to_path_buf(),
            source,
        })?;
    let mut normalized = BTreeSet::new();

    for input in input_paths {
        let candidate = if input.is_absolute() {
            input.clone()
        } else {
            canonical_root.join(input)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|source| HashingError::Io {
                path: candidate.clone(),
                source,
            })?;
        let relative = canonical.strip_prefix(&canonical_root).map_err(|_| {
            HashingError::OutsideWorkspace {
                path: canonical.clone(),
                workspace_root: canonical_root.clone(),
            }
        })?;
        let relative = relative
            .to_str()
            .ok_or_else(|| HashingError::NonUtf8Path(relative.to_path_buf()))?;
        normalized.insert(relative.replace('\\', "/"));
    }

    if normalized.is_empty() {
        return Err(HashingError::EmptyFileSet);
    }
    Ok(normalized.into_iter().collect())
}

/// Hash a stored repository-relative file set.
///
/// A one-file attestation uses that file's ordinary SHA-256, making the pin
/// directly comparable with common tools. Multiple files use a versioned,
/// path-framed digest of each file digest so ordering and concatenation cannot
/// create ambiguity.
pub fn sha256_repo_files(
    workspace_root: &Path,
    relative_paths: &[String],
) -> Result<String, HashingError> {
    let inputs: Vec<PathBuf> = relative_paths.iter().map(PathBuf::from).collect();
    let normalized = normalize_repo_paths(workspace_root, &inputs)?;

    if normalized.len() == 1 {
        return sha256_file(&workspace_root.join(&normalized[0]));
    }

    let mut hasher = Sha256::new();
    hasher.update(b"g8-attestation-files-v1\0");
    for relative in normalized {
        let path_bytes = relative.as_bytes();
        hasher.update((path_bytes.len() as u64).to_be_bytes());
        hasher.update(path_bytes);

        let file_pin = sha256_file(&workspace_root.join(&relative))?;
        let digest_hex = file_pin.strip_prefix(SHA256_PREFIX).unwrap_or(&file_pin);
        hasher.update((digest_hex.len() as u64).to_be_bytes());
        hasher.update(digest_hex.as_bytes());
    }

    Ok(format!("{SHA256_PREFIX}{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_exact_file_bytes_with_standard_sha256() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("payload.txt");
        std::fs::write(&path, b"abc").expect("write payload");

        assert_eq!(
            sha256_file(&path).expect("hash payload"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn repo_file_set_hash_is_order_independent() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"a").expect("write a");
        std::fs::write(dir.path().join("b.txt"), b"b").expect("write b");

        let first = sha256_repo_files(dir.path(), &["a.txt".to_string(), "b.txt".to_string()])
            .expect("hash first order");
        let second = sha256_repo_files(dir.path(), &["b.txt".to_string(), "a.txt".to_string()])
            .expect("hash second order");
        assert_eq!(first, second);
    }

    #[test]
    fn normalization_rejects_files_outside_workspace() {
        let root = tempfile::tempdir().expect("root tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, b"outside").expect("write outside");

        let error = normalize_repo_paths(root.path(), &[outside_file])
            .expect_err("outside path must be rejected");
        assert!(matches!(error, HashingError::OutsideWorkspace { .. }));
    }
}
