//! Shared glob-scoping: resolve a `glob`/`exclude_glob` pair (contract-style
//! gitignore-glob strings, relative to `workspace_root`) into a concrete,
//! sorted (deterministic) list of existing file paths.
//!
//! Used by `AstGrepNoMatch`/`AstGrepMatchCount` (ast-grep has no
//! `--glob`/`--exclude` of its own), `RgMatchCount` (scoping beyond what a
//! single `rg --glob` invocation conveniently expresses across multiple
//! patterns), and `BuiltinAlgorithm::VocabularyDrift`.

use std::path::{Path, PathBuf};

pub(crate) fn expand_glob(
    workspace_root: &Path,
    glob: &[String],
    exclude_glob: &[String],
) -> Result<Vec<PathBuf>, String> {
    let mut builder = ignore::overrides::OverrideBuilder::new(workspace_root);
    for g in glob {
        builder
            .add(g)
            .map_err(|e| format!("invalid glob `{g}`: {e}"))?;
    }
    for e in exclude_glob {
        builder
            .add(&format!("!{e}"))
            .map_err(|err| format!("invalid exclude_glob `{e}`: {err}"))?;
    }
    let overrides = builder.build().map_err(|e| e.to_string())?;
    let walker = ignore::WalkBuilder::new(workspace_root)
        .overrides(overrides)
        .hidden(false)
        .build();
    let mut files = Vec::new();
    for entry in walker {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().is_file() {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn relativize(path: &Path, workspace_root: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_glob_finds_files_under_pattern() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
        std::fs::write(dir.path().join("src/b.txt"), "").unwrap();
        let files = expand_glob(dir.path(), &["src/**/*.rs".to_string()], &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.rs"));
    }

    #[test]
    fn expand_glob_respects_exclude() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "").unwrap();
        let files = expand_glob(
            dir.path(),
            &["src/**/*.rs".to_string()],
            &["src/b.rs".to_string()],
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.rs"));
    }

    #[test]
    fn expand_glob_missing_dir_yields_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let files = expand_glob(dir.path(), &["nonexistent/**".to_string()], &[]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn expand_glob_invalid_pattern_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = expand_glob(dir.path(), &["[".to_string()], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn relativize_strips_workspace_root() {
        let root = Path::new("/a/b");
        let p = Path::new("/a/b/c/d.rs");
        assert_eq!(relativize(p, root), "c/d.rs");
    }
}
