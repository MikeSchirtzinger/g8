//! CLI execution context — store resolution, config loading, output mode.
//!
//! Each subcommand handler receives a `Ctx` that provides the resolved store
//! path, parsed config, and the global output mode. This avoids duplicating
//! store-opening logic across handlers.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use g8_core::{ConvergenceSpace, Project, SpaceId};
use g8_store::{RusqliteStore, StoreConnection};

use crate::cli::OutputMode;

/// Resolved execution context for a CLI invocation.
pub struct Ctx {
    /// Output mode (pretty or JSON).
    pub output: OutputMode,
    /// Whether ANSI color is enabled (pretty mode only).
    pub color: bool,
    /// Whether the `--quiet` flag is set.
    pub quiet: bool,
    /// Absolute path to the `.g8/` directory.
    pub g8_dir: PathBuf,
    /// Absolute path to the store file.
    pub store_path: PathBuf,
    /// Directory whose files checks run against. The parent of `g8_dir`,
    /// except in a linked git worktree that borrows the main checkout's
    /// `.g8/`, where it is the worktree's own top level.
    pub project_root: PathBuf,
    /// Parsed `.g8/config.toml` (if present).
    pub config: Option<G8Config>,
}

impl Ctx {
    /// Build a context from global CLI args.
    ///
    /// `store_override` comes from `--store`; `cwd` is usually `std::env::current_dir()`.
    pub fn new(
        output: OutputMode,
        no_color: bool,
        quiet: bool,
        store_override: Option<&Path>,
        cwd: &Path,
    ) -> Result<Self> {
        let G8Layout {
            g8_dir,
            project_root,
        } = resolve_g8_layout(cwd)?;
        let store_path = store_override
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| g8_dir.join("store.db"));

        let config = load_config(&g8_dir).ok();

        Ok(Ctx {
            output,
            color: !no_color,
            quiet,
            g8_dir,
            store_path,
            project_root,
            config,
        })
    }

    /// Build a context for `g8 init` (the `.g8/` dir may not exist yet).
    pub fn for_init(output: OutputMode, no_color: bool, quiet: bool, cwd: &Path) -> Self {
        let g8_dir = cwd.join(".g8");
        let store_path = g8_dir.join("store.db");
        Ctx {
            output,
            color: !no_color,
            quiet,
            g8_dir,
            store_path,
            project_root: cwd.to_path_buf(),
            config: None,
        }
    }

    /// Open and migrate the store, returning a `RusqliteStore`.
    pub fn open_store(&self) -> Result<RusqliteStore> {
        let mut store = RusqliteStore::open(&self.store_path)
            .with_context(|| format!("opening store at {}", self.store_path.display()))?;
        store.migrate().context("running store migrations")?;
        Ok(store)
    }

    /// Returns whether enforcement is on per `.g8/config.toml`.
    pub fn enforcement_on(&self) -> bool {
        self.config
            .as_ref()
            .map(|c| c.g8.enforcement.as_deref() == Some("on"))
            .unwrap_or(false)
    }

    /// Returns the configured space ID, if any.
    pub fn space_id(&self) -> Option<SpaceId> {
        self.config
            .as_ref()
            .and_then(|c| c.g8.space_id.as_ref())
            .and_then(|s| SpaceId::from_string(s.clone()).ok())
    }
}

// ── G8 dir resolution ────────────────────────────────────────────────────────

/// Where the `.g8/` directory is and which directory its checks apply to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct G8Layout {
    pub g8_dir: PathBuf,
    pub project_root: PathBuf,
}

/// Walk from `start` upward looking for `.g8/space.toml`, then `.g8/config.toml`
/// (a legacy `.govern/` directory is accepted at each level).
///
/// `.g8/` is not tracked, so a linked git worktree has none of its own. When
/// the walk reaches a worktree's top level without a match there, the main
/// checkout's `.g8/` (found through the git common dir) is used, and the
/// project root stays the worktree, so checks read the worktree's files
/// against the shared store.
///
/// Falls back to `start/.g8/` if nothing is found.
pub fn resolve_g8_layout(start: &Path) -> Result<G8Layout> {
    let mut dir = start.to_path_buf();
    loop {
        if let Some(g8_dir) = g8_dir_at(&dir) {
            return Ok(G8Layout {
                g8_dir,
                project_root: dir,
            });
        }
        if let Some(main) = main_checkout_of_linked_worktree(&dir) {
            if let Some(g8_dir) = g8_dir_at(&main) {
                return Ok(G8Layout {
                    g8_dir,
                    project_root: dir,
                });
            }
        }
        if !dir.pop() {
            // Reached fs root without finding anything — fall back to cwd.
            return Ok(G8Layout {
                g8_dir: start.join(".g8"),
                project_root: start.to_path_buf(),
            });
        }
    }
}

/// The initialised `.g8/` (or legacy `.govern/`) directly under `dir`, if any.
fn g8_dir_at(dir: &Path) -> Option<PathBuf> {
    // `.g8/` is the current name; `.govern/` is accepted for projects set up
    // before the rename, so an existing store keeps working without a move.
    [".g8", ".govern"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|candidate| {
            candidate.join("space.toml").exists() || candidate.join("config.toml").exists()
        })
}

/// If `dir` is the top level of a linked git worktree, the main checkout's
/// working directory.
///
/// A linked worktree's `.git` is a file, `gitdir: <repo>/.git/worktrees/<name>`,
/// and that directory's `commondir` names the shared git dir. Submodules also
/// use a `.git` file but have no `commondir`, so they are not matched. A bare
/// common dir has no working tree and yields `None`.
fn main_checkout_of_linked_worktree(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    if !dot_git.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = content
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))?
        .trim();
    let gitdir = dir.join(gitdir);
    let commondir = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common = gitdir.join(commondir.trim()).canonicalize().ok()?;
    if common.file_name()? != ".git" {
        return None;
    }
    common.parent().map(Path::to_path_buf)
}

// ── Config ────────────────────────────────────────────────────────────────────

/// Parsed `.g8/config.toml`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct G8Config {
    /// `[g8]` today; `[govern]` is what `govern init` wrote and is still read.
    #[serde(default, alias = "govern")]
    pub g8: G8Section,
    #[serde(default)]
    pub extractor: ExtractorSection,
    #[serde(default)]
    pub ui: UiSection,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct G8Section {
    pub version: Option<String>,
    pub enforcement: Option<String>,
    pub project_name: Option<String>,
    pub project_id: Option<String>,
    pub space_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct ExtractorSection {
    pub languages: Option<Vec<String>>,
    pub ignore_paths: Option<Vec<String>>,
    pub ast_grep_bin: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct UiSection {
    pub default_output: Option<String>,
}

/// Try to load `.g8/config.toml`.
pub fn load_config(g8_dir: &Path) -> Result<G8Config> {
    let path = g8_dir.join("config.toml");
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))
}

/// Write `.g8/config.toml`.
pub fn write_config(g8_dir: &Path, config: &G8Config) -> Result<()> {
    let content = toml::to_string_pretty(config).context("serializing config")?;
    let path = g8_dir.join("config.toml");
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
}

// ── Space / project helpers ───────────────────────────────────────────────────

/// Resolve or create a default `ConvergenceSpace` and `Project` for a directory.
///
/// Used by `init`, `scan`, and `plan new`.
pub fn ensure_space_and_project(
    store: &mut RusqliteStore,
    space_name: &str,
    project_name: &str,
    root: &Path,
) -> Result<(ConvergenceSpace, Project)> {
    use g8_core::{now_millis, ProjectId};

    // Try to get an existing default space first.
    if let Some(space) = store.get_default_space().context("get_default_space")? {
        let projects = store.list_projects(&space.id).context("list_projects")?;
        if let Some(project) = projects.into_iter().find(|p| p.name == project_name) {
            return Ok((space, project));
        }
        // Space exists but project doesn't — create project.
        let project = Project {
            id: ProjectId::derive(&[space.id.as_str(), project_name]),
            space_id: space.id.clone(),
            name: project_name.to_string(),
            description: None,
            root_path: root.to_path_buf(),
            language: None,
            created_at: now_millis(),
            updated_at: now_millis(),
            meta: None,
        };
        store.upsert_project(&project).context("upsert_project")?;
        return Ok((space, project));
    }

    // Create a fresh space + project.
    let space = ConvergenceSpace {
        id: SpaceId::derive(&[&root.to_string_lossy(), space_name]),
        name: space_name.to_string(),
        root_path: root.to_path_buf(),
        created_at: now_millis(),
        updated_at: now_millis(),
        members: vec![],
        meta: None,
    };
    store.init_space(&space).context("init_space")?;

    let project = Project {
        id: ProjectId::derive(&[space.id.as_str(), project_name]),
        space_id: space.id.clone(),
        name: project_name.to_string(),
        description: None,
        root_path: root.to_path_buf(),
        language: None,
        created_at: now_millis(),
        updated_at: now_millis(),
        meta: None,
    };
    store.upsert_project(&project).context("upsert_project")?;

    Ok((space, project))
}

/// Resolve an existing space from the store; error if none.
pub fn require_space(
    store: &RusqliteStore,
    space_id_override: Option<SpaceId>,
) -> Result<ConvergenceSpace> {
    if let Some(id) = space_id_override {
        return store
            .get_space(&id)
            .context("get_space")?
            .with_context(|| format!("space {} not found", id.as_str()));
    }
    store
        .get_default_space()
        .context("get_default_space")?
        .context("no space found in store: run `g8 init` first")
}

/// Try to resolve the default project from config.
pub fn default_project_name(ctx: &Ctx) -> String {
    ctx.config
        .as_ref()
        .and_then(|c| c.g8.project_name.clone())
        .unwrap_or_else(|| {
            ctx.g8_dir
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "project".to_string())
        })
}

/// Bail with a helpful message if `g8 init` hasn't been run.
pub fn require_init(ctx: &Ctx) -> Result<()> {
    if !ctx.g8_dir.exists() {
        bail!(
            ".g8/ not found at {}. Run `g8 init` first.",
            ctx.g8_dir.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }

    /// A main checkout with an initialised `.g8/` and a linked worktree beside it.
    fn repo_with_worktree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().canonicalize().expect("canonical tempdir");
        let main = base.join("main");
        std::fs::create_dir(&main).expect("main dir");
        git(&main, &["init", "-q"]);
        std::fs::write(main.join("README"), "x").expect("write");
        git(&main, &["add", "README"]);
        git(&main, &["commit", "-qm", "init"]);
        std::fs::create_dir(main.join(".g8")).expect(".g8");
        std::fs::write(main.join(".g8/config.toml"), "[g8]\n").expect("config");
        let wt = base.join("wt");
        git(&main, &["worktree", "add", "-q", wt.to_str().unwrap()]);
        (tmp, main, wt)
    }

    #[test]
    fn linked_worktree_uses_main_checkout_g8_and_keeps_its_own_root() {
        let (_tmp, main, wt) = repo_with_worktree();
        let sub = wt.join("src/deep");
        std::fs::create_dir_all(&sub).expect("subdir");
        let expected = G8Layout {
            g8_dir: main.join(".g8"),
            project_root: wt.clone(),
        };
        assert_eq!(resolve_g8_layout(&wt).unwrap(), expected);
        assert_eq!(resolve_g8_layout(&sub).unwrap(), expected);
    }

    #[test]
    fn worktree_own_g8_takes_precedence() {
        let (_tmp, _main, wt) = repo_with_worktree();
        std::fs::create_dir(wt.join(".g8")).expect(".g8");
        std::fs::write(wt.join(".g8/config.toml"), "[g8]\n").expect("config");
        assert_eq!(
            resolve_g8_layout(&wt).unwrap(),
            G8Layout {
                g8_dir: wt.join(".g8"),
                project_root: wt.clone(),
            }
        );
    }

    #[test]
    fn main_checkout_resolves_as_before() {
        let (_tmp, main, _wt) = repo_with_worktree();
        assert_eq!(
            resolve_g8_layout(&main).unwrap(),
            G8Layout {
                g8_dir: main.join(".g8"),
                project_root: main.clone(),
            }
        );
    }

    #[test]
    fn git_file_without_commondir_is_not_a_worktree() {
        // Submodule shape: `.git` is a file, but its gitdir has no `commondir`.
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().canonicalize().expect("canonical tempdir");
        let modules = base.join("super/.git/modules/sub");
        std::fs::create_dir_all(&modules).expect("modules");
        std::fs::create_dir(base.join("super/.g8")).expect(".g8");
        std::fs::write(base.join("super/.g8/config.toml"), "[g8]\n").expect("config");
        let sub = base.join("elsewhere/sub");
        std::fs::create_dir_all(&sub).expect("sub");
        std::fs::write(sub.join(".git"), format!("gitdir: {}\n", modules.display()))
            .expect("git file");
        assert_eq!(
            resolve_g8_layout(&sub).unwrap(),
            G8Layout {
                g8_dir: sub.join(".g8"),
                project_root: sub.clone(),
            }
        );
    }

    #[test]
    fn load_config_reads_legacy_govern_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("config.toml"),
            "[govern]\nversion = \"0.1.0\"\nproject_name = \"steward\"\nspace_id = \"abc\"\n\n[extractor]\n\n[ui]\n",
        )
        .expect("write");
        let config = load_config(dir.path()).expect("legacy config parses");
        assert_eq!(config.g8.space_id.as_deref(), Some("abc"));
        assert_eq!(config.g8.project_name.as_deref(), Some("steward"));
    }

    #[test]
    fn load_config_reads_current_g8_table() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("config.toml"), "[g8]\nspace_id = \"xyz\"\n")
            .expect("write");
        let config = load_config(dir.path()).expect("config parses");
        assert_eq!(config.g8.space_id.as_deref(), Some("xyz"));
    }
}
