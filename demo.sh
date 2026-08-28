#!/usr/bin/env bash
# demo.sh — end-to-end demo for the G8
#
# Demonstrates: init -> scan -> plan new -> status -> check -> merge
# Uses tempdirs throughout; never touches examples/ or any user state.
# Output goes to stdout.
#
# Requirements:
#   - g8 binary on PATH or built at target/debug/g8 in this workspace
#   - ast-grep on PATH (optional; scan falls back gracefully if absent)
#
# Usage:
#   ./demo.sh                  # run interactively
#   ./demo.sh 2>&1 | tee demo-output.txt   # capture output

set -euo pipefail

# ── Locate g8 binary ────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Prefer a release build, fall back to debug, then PATH.
if [[ -f "$SCRIPT_DIR/target/release/g8" ]]; then
    G8="$SCRIPT_DIR/target/release/g8"
elif command -v cargo &>/dev/null; then
    # Find cargo's target directory (honours CARGO_TARGET_DIR).
    TARGET_DIR=$(cd "$SCRIPT_DIR" && cargo metadata --format-version 1 2>/dev/null \
        | python3 -c "import json,sys; print(json.load(sys.stdin)['target_directory'])" 2>/dev/null \
        || echo "$SCRIPT_DIR/target")
    if [[ -f "$TARGET_DIR/debug/g8" ]]; then
        G8="$TARGET_DIR/debug/g8"
    elif [[ -f "$TARGET_DIR/release/g8" ]]; then
        G8="$TARGET_DIR/release/g8"
    else
        echo "Building g8 (debug)..." >&2
        (cd "$SCRIPT_DIR" && cargo build -q 2>&1) >&2
        G8="$TARGET_DIR/debug/g8"
    fi
else
    G8=$(command -v g8 2>/dev/null || true)
    if [[ -z "$G8" ]]; then
        echo "ERROR: g8 binary not found. Run 'cargo install g8' or build from source." >&2
        exit 1
    fi
fi

EXAMPLES="$SCRIPT_DIR/examples"

# ── Setup tempdirs ───────────────────────────────────────────────────────────

MAIN_DIR=$(mktemp -d)
MERGE_DIR=$(mktemp -d)

cleanup() {
    rm -rf "$MAIN_DIR" "$MERGE_DIR"
}
trap cleanup EXIT

# ── Helper ───────────────────────────────────────────────────────────────────

section() {
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  $1"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

# ── Demo begins ──────────────────────────────────────────────────────────────

echo "g8 v0.1 — G8 demo"
echo "Binary: $G8"
echo "Workspace: $SCRIPT_DIR"

# ── Step 1: init ─────────────────────────────────────────────────────────────

section "Step 1: g8 init — create .g8/ and run AUDIT scan"

cd "$MAIN_DIR"
"$G8" init 2>&1

# ── Step 2: scan the rust-service example ────────────────────────────────────

section "Step 2: g8 scan examples/rust-service — extract capabilities + intents"

cd "$MAIN_DIR"
"$G8" scan "$EXAMPLES/rust-service" 2>&1

echo ""
echo "  (ast-grep extracted 4 capabilities and 4 intents from AGENTS.md)"

# ── Step 3: scan ts-frontend ─────────────────────────────────────────────────

section "Step 3: g8 scan examples/ts-frontend — TypeScript + sidecar .g8.md"

cd "$MAIN_DIR"
"$G8" scan "$EXAMPLES/ts-frontend" 2>&1

# ── Step 4: g8 plan new — feature planning ──────────────────────────────────

section "Step 4: g8 plan new — check fit before starting a new feature"

cd "$MAIN_DIR"
echo "  Query: g8 plan new \"streaming-export\" --substrate \"http-ingestion\" --json"
echo ""
"$G8" plan new "streaming-export" --substrate "http-ingestion" --json 2>&1

# ── Step 5: g8 status ───────────────────────────────────────────────────────

section "Step 5: g8 status — view intent summary"

cd "$MAIN_DIR"
"$G8" status 2>&1

# ── Step 6: g8 check --json ─────────────────────────────────────────────────

section "Step 6: g8 check --json — pairing gate (enforcement off)"

cd "$MAIN_DIR"
"$G8" check --json 2>&1

echo ""
echo "  Enforcement is off by default. Flip it on with: g8 init --enforce"

# ── Step 7: g8 merge — cross-repo conflict detection ────────────────────────

section "Step 7: g8 merge — detecting capability-name collision across repos"

echo "  Setting up repoA (API gateway) and repoB (background workers)..."
echo "  Both declare 'http-fetch' — collision expected."
echo ""

# repoA
REPO_A_DIR=$(mktemp -d)
REPO_B_DIR=$(mktemp -d)

(
    cd "$REPO_A_DIR"
    "$G8" init 2>/dev/null
    "$G8" scan "$EXAMPLES/multi-repo-space/repoA" 2>/dev/null
)

(
    cd "$REPO_B_DIR"
    "$G8" init 2>/dev/null
    "$G8" scan "$EXAMPLES/multi-repo-space/repoB" 2>/dev/null
)

cd "$REPO_A_DIR"
echo "  Running: g8 merge --from <repoB>"
echo ""
"$G8" --output json merge --from "$REPO_B_DIR" 2>&1

echo ""
echo "  Resolve with: g8 link --canonical \"repoA::http-fetch\" --alias \"repoB::http-fetch\""

rm -rf "$REPO_A_DIR" "$REPO_B_DIR"

# ── Done ─────────────────────────────────────────────────────────────────────

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Demo complete. See README.md for next steps."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
