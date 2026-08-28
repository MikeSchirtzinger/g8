# standing-agent, action receipts walkthrough

A worked example of `g8 check --receipt <path>`, the pre-act gate for
**standing agents** (backend contract addendum:
`specs/obligation-checker-contract.md`).

A **session agent** borrows your authority for one supervised task. A
**standing agent** holds durable, bounded, auditable authority: this
directory's `specs/obligations-v0.1.json` is its constitution, and
`g8 check --receipt <run>.json --enforce` is the gate it must clear
*before* a run's proposed actions are allowed to stand, the same choke
point `check --enforce` already provides at pre-commit, moved one step
earlier, to pre-act.

## What's here

- `specs/obligations-v0.1.json`, five `action_receipt`-wired obligations,
  each a `receipt_query` check over a hypothetical "content-ops" agent's run
  receipt:
  - `OBL-PROVENANCE-01`, every `publish` action must carry a `source_ref`
    (op: `absent`).
  - `OBL-ALLOWLIST-01`, every `publish` action's `target.domain` must be one
    of two owned domains (op: `not_in`).
  - `OBL-SPENDCAP-01`, total `spend` across the run must not exceed 500
    (aggregate: `sum`, expectation: `at_most`).
  - `OBL-RATECAP-01`, at most 3 `post` actions per run (aggregate: `count`,
    expectation: `at_most`).
  - `OBL-ESCALATION-01`, every action marked `risky: true` must also carry
    `escalate: true` (**advisory**: visible if violated, never blocks).
- `receipts/clean-run.json`, a run that clears all five.
- `receipts/violating-run.json`, a run that violates all four non-advisory
  gates plus the advisory one, for a fully worked failure case.

The receipt shape (`{ "meta": {...}, "actions": [...] }`) is a convention of
this example, not something `g8` itself defines, a receipt is any JSON
document, and obligations address it with explicit `$.`-prefixed paths (see
the contract addendum for the full path/op grammar).

## Walkthrough

Run these from inside this directory (`examples/standing-agent/`):

```bash
g8 init --no-claude-import --no-subagents
g8 ratify
```

`ratify` hash-pins `specs/obligations-v0.1.json` into `g8.lock`, a
receipt-mode run still enforces ratification (an agent cannot loosen its own
leash mid-run by silently editing its own obligations).

### 1. A clean run, `--enforce` exits 0

```bash
g8 check --receipt receipts/clean-run.json --enforce
echo "exit: $?"
```

```
exit: 0
```

The JSON on stdout carries the self-evidencing decision record, the
checked file, hashed once, plus the scope marker, and every obligation
resolves `passed` at `checked` trust (structural analysis is what
`receipt_query` honestly is; see the addendum's rigor ruling for why
`checked` alone still clears the receipt-mode floor):

```jsonc
{
  "exit_code": 0,
  "errors": [],
  "enforcement_failures": [],
  "receipt": {
    "path": "receipts/clean-run.json",
    "sha256": "sha256:7dc8b9b0fca71dc018a525c5b3df2e34c3354d5b2a3f5f70c8d57738f0a50e76",
    "scope": "action_receipt"
  },
  "obligations": [
    { "id": "OBL-PROVENANCE-01", "status": "passed", "trust": "checked", "...": "..." },
    { "id": "OBL-ALLOWLIST-01",  "status": "passed", "trust": "checked", "...": "..." },
    { "id": "OBL-SPENDCAP-01",   "status": "passed", "trust": "checked", "...": "..." },
    { "id": "OBL-RATECAP-01",    "status": "passed", "trust": "checked", "...": "..." },
    { "id": "OBL-ESCALATION-01", "status": "passed", "trust": "checked", "...": "..." }
  ]
}
```

### 2. A violating run, `--enforce` exits 1

```bash
g8 check --receipt receipts/violating-run.json --enforce
echo "exit: $?"
```

```
exit: 1
```

`enforcement_failures` names every violated **non-advisory** gate as
`receipt_violation` (not `unaccounted_drift`, that classification means the
repo diverged; this means a proposed *act* violated the contract):

```jsonc
"enforcement_failures": [
  { "classification": "receipt_violation", "obligation_id": "OBL-PROVENANCE-01", "detail": "..." },
  { "classification": "receipt_violation", "obligation_id": "OBL-ALLOWLIST-01",  "detail": "..." },
  { "classification": "receipt_violation", "obligation_id": "OBL-SPENDCAP-01",   "detail": "..." },
  { "classification": "receipt_violation", "obligation_id": "OBL-RATECAP-01",   "detail": "..." }
]
```

`OBL-ESCALATION-01` is **not** in that list, its `signal.advisory: true`
means it stays visible in `obligations` (`"status": "failed"`, right there in
the same JSON) but never trips the exit code. Escalation is queue-and-continue
(a decision inbox), never block-and-wait: this run's exit code `1` from the
four hard gates is itself the enqueue signal for a human to look at all five
findings, escalation included.

### 3. Repo mode ignores this file's obligations entirely

```bash
g8 check --json
```

Every obligation here reports `"status": "unknown"` with
`"evidence": {"detail": {"scope": "action_receipt"}, ...}`, repo mode's
choke point is pre-commit, not pre-act, so these are neither executed nor
allowed to affect repo mode's exit code. (This example directory has no
repo-wired obligations of its own, so repo mode's own gate audit is
otherwise empty, the point being demonstrated is that these five don't
leak into it, not that repo mode has something else to pass.)

### 4. Bad input is a refusal, not a crash

```bash
g8 check --receipt receipts/does-not-exist.json --enforce
echo "exit: $?"   # 1, classification invalid_receipt, never 2
g8 check --receipt receipts/does-not-exist.json
echo "exit: $?"   # 0, same finding, just advisory without --enforce
```

## Why this is the "pre-act gate"

`g8` already had the trust model: **ratify** (a human hash-pins the
contract), **attest** (evidence hash-pinned, staleness content-based),
**drift** (divergence named and attributed). Today's only subject was the
repository, checked at `check --enforce`'s pre-commit choke point. This
example is that same machinery pointed at a *proposed or executed run* , 
checked at the tool-call boundary instead, with identical exit-code
semantics (`0` clean, `1` gate failure, `2` reserved for `g8`'s own
internal failures, never a masked bad-input case). After a run, the executed
receipt can also be pinned into the durable ledger the normal way, `g8
attest` against one of this artifact's own obligation IDs, using the receipt
file itself as the attested evidence:

```bash
g8 attest OBL-PROVENANCE-01 --files receipts/clean-run.json
```

Later edits to that file go `stale_attestation_pin`, the same way any other
attested evidence would. (`attest` requires `obligation_id` to already name
an obligation in `specs/obligations-v0.1.json`, it pins evidence for an
existing obligation, it does not mint a new one. The design spec's own
`STANDING-RUN-<id>` phrasing reads as if an arbitrary ID works standalone;
verified live, it does not, a real, disclosed gap between the spec's prose
and `g8 attest`'s current behavior, not something this example silently
routes around.)
