//! `g8 link --canonical <REF> --alias <REF>` — declare cross-project capability identity.
//!
//! REF format: `<project>::<name>` (per ARCH §9 and SPEC Decision 3).

use anyhow::{bail, Context, Result};

use g8_store::StoreConnection;

use crate::cli::{LinkArgs, OutputMode};
use crate::ctx::{require_init, Ctx};
use crate::render::{json, pretty};

pub fn run(ctx: &Ctx, args: &LinkArgs) -> Result<i32> {
    require_init(ctx)?;

    validate_ref(&args.canonical)?;
    validate_ref(&args.alias)?;

    let mut store = ctx.open_store()?;
    store
        .add_capability_alias(&args.canonical, &args.alias)
        .context("add_capability_alias")?;

    let msg = format!("Alias registered: '{}' -> '{}'", args.alias, args.canonical);
    match ctx.output {
        OutputMode::Json => json::render_ok(msg),
        OutputMode::Pretty => {
            if !ctx.quiet {
                pretty::ok(&msg, ctx.color);
            }
        }
    }
    Ok(0)
}

/// Validate that a capability REF is in `<project>::<name>` form.
fn validate_ref(r: &str) -> Result<()> {
    if !r.contains("::") {
        bail!("REF must be in '<project>::<name>' form, got: '{r}'");
    }
    Ok(())
}
