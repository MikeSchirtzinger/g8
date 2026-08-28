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
        let g8_dir = resolve_g8_dir(cwd)?;
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

/// Walk from `start` upward looking for `.g8/space.toml`, then `.g8/config.toml`
/// (a legacy `.govern/` directory is accepted at each level).
/// Returns the directory containing the first `.g8/` match, joining `.g8/`.
///
/// Falls back to `start/.g8/` if neither is found.
pub fn resolve_g8_dir(start: &Path) -> Result<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        // `.g8/` is the current name; `.govern/` is accepted for projects set up
        // before the rename, so an existing store keeps working without a move.
        for name in [".g8", ".govern"] {
            let candidate = dir.join(name);
            if candidate.join("space.toml").exists() || candidate.join("config.toml").exists() {
                return Ok(candidate);
            }
        }
        if !dir.pop() {
            // Reached fs root without finding anything — fall back to cwd.
            return Ok(start.join(".g8"));
        }
    }
}

// ── Config ────────────────────────────────────────────────────────────────────

/// Parsed `.g8/config.toml`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct G8Config {
    #[serde(default)]
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
