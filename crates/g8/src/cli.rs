//! `clap` derive CLI definition — VERBATIM per ARCHITECTURE.md §9.
//!
//! The v0.1 tree follows §9; `attest` and `ratify` are the explicitly approved
//! governance-gap additions from o-g8-gap-20260716.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

// ── Top-level CLI ─────────────────────────────────────────────────────────────

/// G8 — multi-project planning coordination tool.
#[derive(Parser, Debug)]
#[command(
    name = "g8",
    version,
    about = "G8: coordinate plans across projects",
    long_about = None,
)]
pub struct Cli {
    /// Output mode (default: pretty)
    #[arg(long, value_enum, default_value = "pretty", global = true)]
    pub output: OutputMode,

    /// Suppress non-error output (still emits JSON when --output=json)
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace)
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Override .g8/store.db location (default: ./.g8/store.db)
    #[arg(long, global = true, value_name = "PATH")]
    pub store: Option<PathBuf>,

    /// Operate on this space (default: read from .g8/config.toml)
    #[arg(long, global = true, value_name = "ID")]
    pub space: Option<String>,

    /// Disable ANSI color in pretty output
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputMode {
    Pretty,
    Json,
}

// ── Subcommand tree ───────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize g8 in the current directory
    Init(InitArgs),
    /// Scan a directory tree and populate the store
    Scan(ScanArgs),
    /// Plan lifecycle management
    Plan(PlanCmd),
    /// Show INTENT_SUMMARY; regenerate .g8/INTENT_SUMMARY.md
    Status(StatusArgs),
    /// Run pairing-gate; emit errors
    Check(CheckArgs),
    /// Record hash-pinned evidence for an obligation
    Attest(AttestArgs),
    /// Ratify the current obligations specification into g8.lock
    Ratify,
    /// Cross-space conflict detection
    Merge(MergeArgs),
    /// Ad-hoc query against the store
    Query(QueryArgs),
    /// Manage space federation
    Space(SpaceCmd),
    /// Declare cross-project capability identity
    Link(LinkArgs),
    /// Manage substrates (Vocabulary Council pattern)
    Substrate(SubstrateCmd),
    /// Serve a live web dashboard of the convergence space (see ARCHITECTURE.md §9)
    ///
    /// Requires the `serve` build feature (off by default — see
    /// crates/g8/Cargo.toml). Not compiled at all otherwise: no tiny_http
    /// dependency, no entry in `--help`.
    #[cfg(feature = "serve")]
    Serve(ServeArgs),
}

// ── attest / ratify ─────────────────────────────────────────────────────────

/// Record fresh, content-hash-pinned evidence for an obligation.
#[derive(Args, Debug)]
pub struct AttestArgs {
    /// Obligation ID from specs/obligations-v0.1.json
    #[arg(value_name = "OBLIGATION-ID")]
    pub obligation_id: String,

    /// Files inspected as evidence (repository-relative or absolute under root)
    #[arg(long, required = true, value_name = "PATH", num_args = 1..)]
    pub files: Vec<PathBuf>,

    /// Human-auditable evidence pointer (defaults to the attested file list)
    #[arg(long, alias = "evidence-pointer", value_name = "POINTER")]
    pub evidence: Option<String>,
}

// ── serve ───────────────────────────────────────────────────────────────────
// Gated behind the `serve` feature end-to-end (this struct, the `Command`
// variant above, `mod serve` + its dispatch arm in main.rs). See
// crates/g8/src/serve/mod.rs for the hardening this args struct exists
// to configure (host guard, token auth, origin-scoped CORS).

/// Serve the live convergence-space dashboard over HTTP.
#[cfg(feature = "serve")]
#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Port to bind (default: 8799)
    #[arg(long, default_value_t = 8799)]
    pub port: u16,

    /// Host/interface to bind (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Don't open the dashboard in a browser on start
    #[arg(long)]
    pub no_open: bool,

    /// Base URL of a running AgentViz explorer for the "Open in AgentViz" deep-link
    #[arg(long, value_name = "URL", default_value = "http://localhost:8090")]
    pub agentviz_url: String,

    /// Bind a non-loopback host (0.0.0.0, a LAN IP, ...) despite the risk.
    ///
    /// `g8 serve` only protects its mutating routes with an ephemeral token
    /// printed to this terminal — there's no TLS and no real auth. Exposing
    /// it beyond loopback hands anyone who can reach the port a shot at that
    /// token. Pass this only if you accept that (e.g. a trusted LAN, or
    /// you're fronting it with your own auth/TLS proxy).
    #[arg(long)]
    pub allow_external_unsafe: bool,
}

// ── init ──────────────────────────────────────────────────────────────────────

/// Initialize g8 in the current directory.
#[derive(Args, Debug)]
pub struct InitArgs {
    /// Set enforcement = on (default: off; AUDIT-before-enforce)
    #[arg(long)]
    pub enforce: bool,

    /// Install .git/hooks/pre-commit calling `g8 check`
    #[arg(long)]
    pub install_pre_commit: bool,

    /// Don't touch CLAUDE.md
    #[arg(long)]
    pub no_claude_import: bool,

    /// Don't drop .claude/agents/*.md
    #[arg(long)]
    pub no_subagents: bool,

    /// Space name (default: directory basename)
    #[arg(long, value_name = "STR")]
    pub name: Option<String>,
}

// ── scan ──────────────────────────────────────────────────────────────────────

/// Scan a directory tree and populate the store.
#[derive(Args, Debug)]
pub struct ScanArgs {
    /// Directory to scan (default: current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Override project name (default: from .g8/config.toml)
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,

    /// Print AUDIT report; don't write to store
    #[arg(long)]
    pub report_only: bool,

    /// Don't enable file-watch after scan completes
    #[arg(long)]
    pub no_watch: bool,
}

// ── plan ──────────────────────────────────────────────────────────────────────

/// Plan lifecycle subcommands.
#[derive(Args, Debug)]
pub struct PlanCmd {
    #[command(subcommand)]
    pub subcommand: PlanSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum PlanSubcommand {
    /// Create a new plan; runs planner_intent_check
    New(PlanNewArgs),
    /// List plans in the current space
    List(PlanListArgs),
    /// Show one plan in detail
    Show(PlanShowArgs),
    /// Move plan to Parked
    Park(PlanParkArgs),
    /// Move parked plan to Scoped (re-runs planner)
    Promote(PlanPromoteArgs),
    /// Idea -> Scoped (runs planner)
    Scope(PlanScopeArgs),
    /// Scoped -> Dispatched (WIP cap check)
    Dispatch(PlanDispatchArgs),
    /// Dispatched -> Blocked
    Block(PlanBlockArgs),
    /// Blocked -> Dispatched
    Unblock(PlanUnblockArgs),
    /// Dispatched -> Done
    Complete(PlanCompleteArgs),
}

#[derive(Args, Debug)]
pub struct PlanNewArgs {
    /// Plan title
    #[arg(value_name = "TITLE")]
    pub title: String,

    /// Primary substrate for the plan
    #[arg(long, value_name = "NAME")]
    pub substrate: Option<String>,

    /// Plan description
    #[arg(long, value_name = "STR")]
    pub description: Option<String>,

    /// Capability names this plan touches (repeatable)
    #[arg(long, value_name = "NAME", num_args = 1..)]
    pub touches: Option<Vec<String>>,

    /// Create directly as Parked
    #[arg(long)]
    pub park: bool,

    /// Create as Dispatched (subject to WIP cap)
    #[arg(long)]
    pub dispatch: bool,

    /// Output FitReport as JSON (equiv: --output json)
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct PlanListArgs {
    /// Filter by status (repeatable)
    #[arg(long, value_name = "STATUS", num_args = 1..)]
    pub status: Option<Vec<String>>,

    /// Filter by substrate
    #[arg(long, value_name = "NAME")]
    pub substrate: Option<String>,

    /// Filter by project
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,

    /// Shortcut for --status Parked
    #[arg(long)]
    pub parked: bool,
}

#[derive(Args, Debug)]
pub struct PlanShowArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct PlanParkArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,

    /// Parked reason
    #[arg(long, value_name = "STR")]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct PlanPromoteArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct PlanScopeArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct PlanDispatchArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct PlanBlockArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,

    /// Blocked reason
    #[arg(long, value_name = "STR")]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct PlanUnblockArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct PlanCompleteArgs {
    /// Plan ID
    #[arg(value_name = "ID", allow_hyphen_values = true)]
    pub id: String,
}

// ── status ────────────────────────────────────────────────────────────────────

/// Show INTENT_SUMMARY; regenerate .g8/INTENT_SUMMARY.md.
#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Don't regenerate the file (just print)
    #[arg(long)]
    pub no_regen: bool,

    /// Include detected intra-space conflicts in output
    #[arg(long)]
    pub intra_conflicts: bool,
}

// ── check ─────────────────────────────────────────────────────────────────────

/// Run pairing-gate; emit errors.
#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Equivalent to --output json
    #[arg(long)]
    pub json: bool,

    /// Enforce findings for this invocation without changing config.toml
    #[arg(long)]
    pub enforce: bool,

    /// Fail on warnings as well as errors
    #[arg(long)]
    pub strict: bool,

    /// Check just this project (default: all)
    #[arg(long, value_name = "NAME")]
    pub project: Option<String>,

    /// Check an action receipt instead of the repository — the pre-act gate
    /// for standing agents. Only
    /// `action_receipt`-wired obligations run, against the parsed receipt;
    /// repo-wired obligations and the capability pairing check are skipped
    /// (different choke point). Ratification is still enforced.
    #[arg(long, value_name = "PATH")]
    pub receipt: Option<PathBuf>,
}

// ── merge ─────────────────────────────────────────────────────────────────────

/// Cross-space conflict detection.
#[derive(Args, Debug)]
pub struct MergeArgs {
    /// Path to remote space root (must contain .g8/)
    #[arg(long, required = true, value_name = "PATH")]
    pub from: String,

    /// Equivalent to --output json
    #[arg(long)]
    pub json: bool,

    /// Filter by conflict kind (repeatable)
    #[arg(long, value_name = "KIND", num_args = 1..)]
    pub kind: Vec<String>,
}

// ── query ─────────────────────────────────────────────────────────────────────

/// Ad-hoc query against the store.
///
/// Supported forms: plans:<status>, capabilities [--substrate=X],
/// intents [--path=X], bottlenecks [--substrate=X],
/// drift [--substrate=X], parked [--like=X]
#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Query DSL expression
    #[arg(value_name = "QUERY-DSL")]
    pub query: String,
}

// ── space ─────────────────────────────────────────────────────────────────────

/// Manage space federation.
#[derive(Args, Debug)]
pub struct SpaceCmd {
    #[command(subcommand)]
    pub subcommand: SpaceSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum SpaceSubcommand {
    /// Add a project to the current space
    Add(SpaceAddArgs),
    /// Remove a project from the space
    Remove(SpaceRemoveArgs),
    /// List members of the current space
    List,
}

#[derive(Args, Debug)]
pub struct SpaceAddArgs {
    /// Path to the project directory
    #[arg(value_name = "PATH")]
    pub path: String,

    /// Override project name
    #[arg(long, value_name = "STR")]
    pub name: Option<String>,
}

#[derive(Args, Debug)]
pub struct SpaceRemoveArgs {
    /// Project name to remove
    #[arg(value_name = "PROJECT")]
    pub project: String,
}

// ── link ──────────────────────────────────────────────────────────────────────

/// Declare cross-project capability identity.
///
/// REF format: `<project>::<name>`
#[derive(Args, Debug)]
pub struct LinkArgs {
    /// Canonical capability reference (<project>::<name>)
    #[arg(long, required = true, value_name = "REF")]
    pub canonical: String,

    /// Alias capability reference (<project>::<name>)
    #[arg(long, required = true, value_name = "REF")]
    pub alias: String,
}

// ── substrate ─────────────────────────────────────────────────────────────────

/// Manage substrates (Vocabulary Council pattern).
#[derive(Args, Debug)]
pub struct SubstrateCmd {
    #[command(subcommand)]
    pub subcommand: SubstrateSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum SubstrateSubcommand {
    /// Register a substrate
    Add(SubstrateAddArgs),
    /// List substrates with budgets and current WIP
    List,
}

#[derive(Args, Debug)]
pub struct SubstrateAddArgs {
    /// Substrate name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Override default WIP cap (default: 3)
    #[arg(long, value_name = "N")]
    pub wip_cap: Option<u32>,

    /// Override stale threshold in days (default: 14)
    #[arg(long, value_name = "N")]
    pub stale_days: Option<u32>,
}
