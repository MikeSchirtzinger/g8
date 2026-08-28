//! Pretty (human-readable Markdown-like) output renderer.
//!
//! Writes to stdout. ANSI colour is used when `--no-color` is NOT set and the
//! output is a terminal. All functions accept a `color` bool derived from the
//! global `--no-color` flag (inverted: `color = !no_color`).

use g8_core::{Conflict, ConflictKind, FitReport, Plan, PlanStatus, Recommendation, Severity};
use g8_obligations::{ObligationResult, ObligationStatus};
use g8_store::{PairingError, PairingErrorReason};

// ── Colour helpers ────────────────────────────────────────────────────────────

fn green(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[32m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn yellow(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[33m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn red(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[31m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn bold(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[1m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn dim(s: &str, color: bool) -> String {
    if color {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

// ── FitReport ─────────────────────────────────────────────────────────────────

/// Render a `FitReport` to stdout.
pub fn render_fit_report(report: &FitReport, color: bool) {
    println!("{}", bold("## FitReport", color));
    println!("Plan: {}", report.draft.title);
    if let Some(ref sub) = report.draft.substrate {
        println!("Substrate: {sub}");
    }
    println!();

    // Recommendation
    let rec_str = format!("{:?}", report.recommendation);
    let rec_colored = match report.recommendation {
        Recommendation::Proceed => green(&rec_str, color),
        Recommendation::Park | Recommendation::Wait => yellow(&rec_str, color),
        Recommendation::Drop | Recommendation::Pivot => red(&rec_str, color),
        Recommendation::Extend | Recommendation::Rename => yellow(&rec_str, color),
    };
    println!("{}", bold(&format!("Recommendation: {rec_colored}"), color));
    println!();

    // Budget
    if let Some(ref budget) = report.budget_status {
        let status = if budget.at_cap {
            red("AT CAP", color)
        } else {
            green("OK", color)
        };
        println!(
            "WIP Budget [{status}]: {}/{} active on substrate '{}'",
            budget.wip_current, budget.wip_cap, budget.substrate
        );
        println!();
    }

    // Existing matches
    if !report.existing_matches.is_empty() {
        println!("{}", bold("Existing Matches:", color));
        for m in &report.existing_matches {
            println!(
                "  - [{:?}] {} ({})",
                m.status,
                m.title,
                dim(&format!("{:?}", m.match_kind), color)
            );
        }
        println!();
    }

    // Bottlenecks
    if !report.bottlenecks.is_empty() {
        println!("{}", bold("Bottlenecks:", color));
        for b in &report.bottlenecks {
            let days = b
                .days_in_flight
                .map(|d| format!(": {d}d in flight"))
                .unwrap_or_default();
            let reason = b
                .blocked_reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            println!("  - [{:?}] {}{}{}", b.status, b.title, days, reason);
        }
        println!();
    }

    // Drift hints
    if !report.drift_hints.is_empty() {
        println!("{}", bold("Drift Hints:", color));
        for d in &report.drift_hints {
            println!("  - {}: {}d stale", d.title, d.days_stale);
        }
        println!();
    }

    // Intent overlaps
    if !report.intent_overlaps.is_empty() {
        println!("{}", bold("Intent Overlaps:", color));
        for i in &report.intent_overlaps {
            let proj = i
                .project
                .as_deref()
                .map(|p| format!(" [{p}]"))
                .unwrap_or_default();
            println!("  - {}{}", i.title, proj);
        }
        println!();
    }

    // Parked ideas
    if !report.parked_ideas.is_empty() {
        println!("{}", bold("Parked Ideas (related):", color));
        for p in &report.parked_ideas {
            let reason = p
                .parked_reason
                .as_deref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default();
            println!("  - {}{}", p.title, reason);
        }
        println!();
    }
}

// ── Plan list ─────────────────────────────────────────────────────────────────

/// Render a list of plans.
pub fn render_plan_list(plans: &[Plan], color: bool) {
    if plans.is_empty() {
        println!("{}", dim("No plans found.", color));
        return;
    }
    println!("{}", bold("Plans:", color));
    println!(
        "{}",
        dim(
            "  ID                     | Status      | Substrate       | Title",
            color
        )
    );
    println!(
        "{}",
        dim(
            "  -----------------------|-------------|-----------------|------",
            color
        )
    );
    for plan in plans {
        let id_short = &plan.id.as_str()[..plan.id.as_str().len().min(8)];
        let status_str = format_status(&plan.status, color);
        let substrate = plan.substrate.as_deref().unwrap_or("-");
        println!(
            "  {id_short:<23} | {status_str:<11} | {substrate:<15} | {}",
            plan.title
        );
    }
}

/// Render a single plan in detail.
pub fn render_plan_detail(plan: &Plan, color: bool) {
    println!("{}", bold(&format!("## Plan: {}", plan.title), color));
    println!("ID:        {}", plan.id.as_str());
    println!("Status:    {}", format_status(&plan.status, color));
    if let Some(ref sub) = plan.substrate {
        println!("Substrate: {sub}");
    }
    if let Some(ref desc) = plan.description {
        println!("Description: {desc}");
    }
    if let Some(ref reason) = plan.parked_reason {
        println!("Parked reason: {reason}");
    }
    if let Some(ref reason) = plan.blocked_reason {
        println!("Blocked reason: {reason}");
    }
    println!("WIP weight: {}", plan.wip_weight);
}

fn format_status(status: &PlanStatus, color: bool) -> String {
    match status {
        PlanStatus::Idea => dim("Idea", color),
        PlanStatus::Scoped => yellow("Scoped", color),
        PlanStatus::Dispatched => green("Dispatched", color),
        PlanStatus::Blocked => red("Blocked", color),
        PlanStatus::Done => green("Done", color),
        PlanStatus::Parked => dim("Parked", color),
    }
}

// ── Check output ──────────────────────────────────────────────────────────────

/// Render pairing-gate errors (pretty mode, human text to stderr).
pub fn render_check_errors(errors: &[PairingError], color: bool) {
    if errors.is_empty() {
        eprintln!("{}", green("g8 check: all capabilities paired.", color));
        return;
    }
    eprintln!(
        "{}",
        red(&format!("{} pairing error(s):", errors.len()), color)
    );
    for e in errors {
        let reason = format_pairing_reason(e.reason);
        eprintln!(
            "  {} {}:{}: {reason}",
            e.capability,
            e.file.display(),
            e.line
        );
    }
}

fn format_pairing_reason(reason: PairingErrorReason) -> &'static str {
    // Delegate to the shared contract enum's own string form rather than
    // re-declaring a second, parallel match arm over the same 4 reasons —
    // `super::to_pairing_reason` is the one mapping site.
    super::to_pairing_reason(reason).as_str()
}

// ── Obligations self-audit ───────────────────────────────────────────────────

/// Render the obligations self-audit summary (pretty mode, human text to
/// stderr) — `specs/obligation-checker-contract.md` §5. Called on every
/// `g8 check` run, even when enforcement is off (audit-before-enforce); an
/// empty slice — the common case for any project that isn't g8's own
/// repository — prints nothing.
pub fn render_obligations(obligations: &[ObligationResult], color: bool) {
    if obligations.is_empty() {
        return;
    }

    let passed = count_status(obligations, ObligationStatus::Passed);
    let failed = count_status(obligations, ObligationStatus::Failed);
    let errored = count_status(obligations, ObligationStatus::Error);
    let unknown = count_status(obligations, ObligationStatus::Unknown);

    eprintln!();
    eprintln!(
        "{}",
        bold(
            &format!(
                "Obligations self-audit: {passed} passed, {failed} failed, \
                 {errored} error, {unknown} unknown"
            ),
            color
        )
    );
    // Keep the noisy common case (passed) out of the per-line human view —
    // the summary line above already accounts for it.
    for o in obligations
        .iter()
        .filter(|o| o.status != ObligationStatus::Passed)
    {
        let status_str = match o.status {
            ObligationStatus::Passed => green("passed", color),
            ObligationStatus::Failed => red("failed", color),
            ObligationStatus::Error => red("error", color),
            ObligationStatus::Unknown => yellow("unknown", color),
        };
        eprintln!("  [{status_str}] {}: {}", o.id, o.evidence.summary);
    }
}

pub fn render_enforcement_failures(failures: &[super::json::EnforcementFailure], color: bool) {
    if failures.is_empty() {
        return;
    }

    eprintln!();
    eprintln!("{}", bold("Enforcement classifications:", color));
    for failure in failures {
        let obligation = failure
            .obligation_id
            .as_deref()
            .map(|id| format!(" {id}"))
            .unwrap_or_default();
        eprintln!(
            "  [{}]{}: {}",
            red(failure.classification.as_str(), color),
            obligation,
            failure.detail
        );
    }
}

fn count_status(obligations: &[ObligationResult], status: ObligationStatus) -> usize {
    obligations.iter().filter(|o| o.status == status).count()
}

// ── Conflict report ───────────────────────────────────────────────────────────

/// Render a list of conflicts from `g8 merge`.
pub fn render_conflicts(conflicts: &[Conflict], color: bool) {
    if conflicts.is_empty() {
        println!("{}", green("No conflicts detected.", color));
        return;
    }
    println!(
        "{}",
        bold(&format!("{} conflict(s) detected:", conflicts.len()), color)
    );
    println!();
    for (i, c) in conflicts.iter().enumerate() {
        let sev = match c.severity {
            Severity::Error => red("ERROR", color),
            Severity::Warn => yellow("WARN", color),
            Severity::Info => dim("INFO", color),
        };
        let kind = match c.kind {
            ConflictKind::DuplicateIntent => "DuplicateIntent",
            ConflictKind::CapabilityNameCollision => "CapabilityNameCollision",
            ConflictKind::ContradictoryDecisions => "ContradictoryDecisions",
            ConflictKind::GovernanceViolation => "GovernanceViolation",
        };
        println!("{}. [{sev}] {kind}", i + 1);
        for ev in &c.evidence {
            println!("   {ev:?}");
        }
        println!();
    }
}

// ── Simple messages ───────────────────────────────────────────────────────────

/// Print a success message.
pub fn ok(msg: &str, color: bool) {
    println!("{}", green(&format!("✓ {msg}"), color));
}

/// Print a warning message.
#[allow(dead_code)] // public renderer API; retained for future warning surfaces
pub fn warn(msg: &str, color: bool) {
    eprintln!("{}", yellow(&format!("⚠ {msg}"), color));
}
