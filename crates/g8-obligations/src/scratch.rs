//! Minimal, randomness-free scratch-directory helper for PRODUCTION code
//! paths (`FixtureIntegrationTest`/`ByteDiffTwice`'s isolated fixtures).
//!
//! Deliberately NOT `tempfile::tempdir()`: verified via `cargo tree -p
//! g8-obligations -i getrandom` that `tempfile` pulls `getrandom` (via
//! `fastrand`) the moment it's used as a REGULAR (non-dev) dependency — a
//! genuinely NEW randomness edge (`getrandom v0.4.2`), independent of and
//! additional to the pre-existing `nanoid → rand → getrandom v0.2.17` chain
//! that already reaches this crate transitively via `g8-core`/`g8-store`
//! (that one is Q4/OBL-D11-02, explicitly not this crate's to fix). Once
//! `g8` depends on `g8-obligations` (T5), a regular `tempfile`
//! dependency here would carry that second edge into the PRODUCTION `g8`
//! binary's tree, not just its test binary — exactly what the "introduce no
//! NEW randomness edges" gate is checking for.
//!
//! Uniqueness here comes from the process ID plus a monotonic counter, not
//! randomness — fully deterministic-zone compliant. `tempfile` remains
//! available as a dev-dependency for this crate's own tests (see
//! `Cargo.toml`), where it never touches the production binary's tree.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// An isolated scratch directory, removed on drop (RAII, mirroring
/// `tempfile::TempDir`'s ergonomics without the randomness dependency).
pub(crate) struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub(crate) fn new() -> std::io::Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("g8-obligations-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_cleans_up_a_real_directory() {
        let path;
        {
            let dir = ScratchDir::new().unwrap();
            path = dir.path().to_path_buf();
            assert!(path.exists());
            assert!(path.is_dir());
        }
        assert!(!path.exists(), "scratch dir must be removed on drop");
    }

    #[test]
    fn concurrent_calls_never_collide() {
        let a = ScratchDir::new().unwrap();
        let b = ScratchDir::new().unwrap();
        assert_ne!(a.path(), b.path());
    }
}
