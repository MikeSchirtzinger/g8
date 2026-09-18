//! `g8` — G8 CLI binary.
//!
//! Entry point: parses global flags, initialises tracing, dispatches to
//! subcommand handlers in `cmd/`. Contains zero domain logic.
//!
//! Tracing is initialised per ARCHITECTURE.md §12:
//! - Logs to stderr (never stdout — JSON output on stdout must stay clean).
//! - Env-filter defaults to `g8=info`; `--verbose` bumps to debug/trace.
//! - `G8_TRACE_JSON=1` switches to JSON-format traces.

use std::process;

use clap::Parser;
use tracing_subscriber::{fmt, EnvFilter};

mod cli;
mod cmd;
mod ctx;
mod render;
#[cfg(feature = "serve")]
mod serve;
mod space_toml;

use cli::{Cli, Command, OutputMode};
use ctx::Ctx;

fn main() {
    let cli = Cli::parse();

    // ── SIGPIPE ──────────────────────────────────────────────────────────────
    // Rust's runtime ignores SIGPIPE at startup, so `g8 check | head` used to
    // die with "failed printing to stdout: Broken pipe" (a panic in
    // `println!`, exit 101). Restoring the default disposition lets a closed
    // pipe end the process quietly, as every other CLI does. `serve` keeps
    // the ignore: a dashboard client closing its socket must not take the
    // server down.
    #[cfg(feature = "serve")]
    let is_serve = matches!(cli.command, Command::Serve(_));
    #[cfg(not(feature = "serve"))]
    let is_serve = false;
    if !is_serve {
        restore_default_sigpipe();
    }

    // ── Tracing init (per ARCH §12) ──────────────────────────────────────────
    init_tracing(cli.verbose);

    // ── Build execution context ───────────────────────────────────────────────
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let ctx = match &cli.command {
        Command::Init(_) => {
            // `init` runs before `.g8/` exists; use the pre-init context builder.
            Ctx::for_init(cli.output, cli.no_color, cli.quiet, &cwd)
        }
        _ => match Ctx::new(
            cli.output,
            cli.no_color,
            cli.quiet,
            cli.store.as_deref(),
            &cwd,
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("g8: error building context: {e:#}");
                process::exit(2);
            }
        },
    };

    // ── Root tracing span for the entire CLI invocation (ARCH §12) ───────────
    let span = tracing::info_span!(
        "cli.command",
        command = ?&cli.command,
    );
    let _guard = span.enter();

    // ── Dispatch ──────────────────────────────────────────────────────────────
    let result = dispatch(&ctx, &cli.command);

    let exit_code = match result {
        Ok(code) => code,
        Err(e) => {
            // Print the full anyhow error chain on stderr (`{e:#}` flattens
            // .context() layers so the user sees the root cause, not just the
            // top-level "running planner" label).
            eprintln!("g8: {e:#}");
            // Emit failure JSON on stdout if in JSON mode so CI consumers always
            // get valid JSON. Crucially, status must be "error" (was "ok" before).
            if ctx.output == OutputMode::Json {
                render::json::render_error(format!("{e:#}"), 2);
            }
            2
        }
    };

    process::exit(exit_code);
}

fn dispatch(ctx: &Ctx, command: &Command) -> anyhow::Result<i32> {
    match command {
        Command::Init(args) => cmd::init::run(ctx, args),
        Command::Scan(args) => cmd::scan::run(ctx, args),
        Command::Plan(plan_cmd) => cmd::plan::run(ctx, &plan_cmd.subcommand),
        Command::Status(args) => cmd::status::run(ctx, args),
        Command::Check(args) => cmd::check::run(ctx, args),
        Command::Attest(args) => cmd::attest::run(ctx, args),
        Command::Ratify => cmd::ratify::run(ctx),
        Command::Merge(args) => cmd::merge::run(ctx, args),
        Command::Query(args) => cmd::query::run(ctx, args),
        Command::Space(space_cmd) => cmd::space::run(ctx, &space_cmd.subcommand),
        Command::Link(args) => cmd::link::run(ctx, args),
        Command::Substrate(sub_cmd) => cmd::substrate::run(ctx, &sub_cmd.subcommand),
        #[cfg(feature = "serve")]
        Command::Serve(args) => serve::run(ctx, args),
    }
}

// ── Tracing initialisation ────────────────────────────────────────────────────

fn init_tracing(verbose: u8) {
    let default_directive = match verbose {
        0 => "g8=warn",
        1 => "g8=info",
        2 => "g8=debug",
        _ => "g8=trace",
    };

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directive));

    if std::env::var("G8_TRACE_JSON").as_deref() == Ok("1") {
        fmt()
            .json()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .compact()
            .init();
    }
}

#[cfg(unix)]
fn restore_default_sigpipe() {
    // SAFETY: signal(2) with SIG_DFL only resets the disposition of SIGPIPE
    // for this process. No handler is installed and no memory is shared.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}
