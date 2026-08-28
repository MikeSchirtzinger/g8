//! `CargoMetadataNoDep` + `CargoMetadataDepGraph` — contract §1.1.
//!
//! Both shell out to `cargo metadata --format-version=1` (never `cargo tree`
//! or anything that would require shell-string parsing) and walk the JSON
//! `resolve` graph directly. `resolve.nodes[].dependencies` already respects
//! Cargo's own feature resolution — verified empirically (a default-config
//! `cargo metadata` on this workspace excludes `tiny_http`/`nanoid` from
//! `g8`'s resolved deps; `--features g8/serve` includes them) —
//! which is exactly what makes `CargoMetadataNoDep` the one backend in the
//! enum that correctly distinguishes "not a dependency" from "not an
//! *active* dependency" (contract §1.1's own claim about this backend).
//!
//! # Dependency-kind scope (Q4 ruling, 2026-07-02)
//!
//! `CargoMetadataNoDep` walks **normal + build** dependency edges only —
//! dev-dependencies are OUT of deny-list scope (they never ship in the
//! binary and cannot affect end-user-observed determinism; build-deps stay
//! in because they execute during artifact production). This resolves the
//! open scoping question contract §2.5 item 4 flagged for Q4: the original
//! obligation's own `jq` check was kind-blind, so `tempfile`'s dev-only
//! `fastrand`/`getrandom` subtree polluted D11-02's evidence. The evidence
//! detail discloses the scope on every run (`dependency_kinds_in_scope`).
//!
//! `CargoMetadataDepGraph` (D8-01's exact-edge-set check) deliberately stays
//! kind-blind — its obligation forbids "any workspace-internal dependency
//! edge not in required_edges", with no shipped/dev carve-out ruled.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::Path;

use regex::Regex;
use serde_json::{json, Value};

use crate::backend::{BuildConfig, CargoMetadataDepGraphArgs, CargoMetadataNoDepArgs, DepMatcher};
use crate::proc::run_tool;
use crate::result::ObligationStatus;

pub(crate) fn no_dep(
    args: &CargoMetadataNoDepArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    no_dep_with_binary(Path::new("cargo"), args, workspace_root)
}

fn no_dep_with_binary(
    cargo_bin: &Path,
    args: &CargoMetadataNoDepArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    let metadata = match fetch_metadata(cargo_bin, workspace_root, &args.build_config) {
        Ok(m) => m,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let graph = match Graph::from_metadata(&metadata) {
        Ok(g) => g,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let roots = graph.resolve_scope_roots(&args.scope_crates);
    if !args.scope_crates.is_empty() && roots.len() != args.scope_crates.len() {
        let found: HashSet<&str> = roots.iter().map(|id| graph.name_of(id)).collect();
        let missing: Vec<&String> = args
            .scope_crates
            .iter()
            .filter(|c| !found.contains(c.as_str()))
            .collect();
        if !missing.is_empty() {
            return (
                ObligationStatus::Error,
                json!({ "error": format!("scope_crates not found in workspace: {missing:?}") }),
            );
        }
    }

    let mut compiled_regexes = Vec::new();
    for m in &args.denied {
        if let DepMatcher::Regex(pat) = m {
            match Regex::new(pat) {
                Ok(r) => compiled_regexes.push(Some(r)),
                Err(e) => {
                    return (
                        ObligationStatus::Error,
                        json!({ "error": format!("invalid regex `{pat}`: {e}") }),
                    )
                }
            }
        } else {
            compiled_regexes.push(None);
        }
    }

    // Q4 ruling: deny-list scope = shipped (normal + build) edges only.
    let reachable = graph.reachable_from(&roots, true);

    let mut matched: Vec<String> = Vec::new();
    let mut via: BTreeMap<String, String> = BTreeMap::new();
    for (id, name) in &reachable {
        for (i, matcher) in args.denied.iter().enumerate() {
            let hit = match matcher {
                DepMatcher::Exact(v) => name == v,
                DepMatcher::Regex(_) => compiled_regexes[i].as_ref().unwrap().is_match(name),
            };
            if hit {
                matched.push(name.clone());
                if let Some(parent) = graph.first_parent_name(id, &roots) {
                    via.insert(name.clone(), parent);
                }
            }
        }
    }
    matched.sort();
    matched.dedup();

    let denied_checked: Vec<String> = args
        .denied
        .iter()
        .map(|m| match m {
            DepMatcher::Exact(v) => v.clone(),
            DepMatcher::Regex(v) => v.clone(),
        })
        .collect();

    // Disclosed on every run (contract §10 culture): dev-dependency edges
    // are out of deny-list scope per the Q4 ruling.
    let kinds_in_scope = json!(["normal", "build"]);

    if matched.is_empty() {
        (
            ObligationStatus::Passed,
            json!({
                "denied_checked": denied_checked,
                "dependency_kinds_in_scope": kinds_in_scope,
            }),
        )
    } else {
        (
            ObligationStatus::Failed,
            json!({
                "matched": matched,
                "via": via,
                "denied_checked": denied_checked,
                "dependency_kinds_in_scope": kinds_in_scope,
            }),
        )
    }
}

pub(crate) fn dep_graph(
    args: &CargoMetadataDepGraphArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    dep_graph_with_binary(Path::new("cargo"), args, workspace_root)
}

fn dep_graph_with_binary(
    cargo_bin: &Path,
    args: &CargoMetadataDepGraphArgs,
    workspace_root: &Path,
) -> (ObligationStatus, Value) {
    let metadata = match fetch_metadata(cargo_bin, workspace_root, &BuildConfig::Default) {
        Ok(m) => m,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    let graph = match Graph::from_metadata(&metadata) {
        Ok(g) => g,
        Err(e) => return (ObligationStatus::Error, json!({ "error": e })),
    };

    // Workspace-member -> workspace-member edges only.
    let mut actual_edges: HashSet<(String, String)> = HashSet::new();
    for id in &graph.workspace_members {
        let from_name = graph.name_of(id).to_string();
        if let Some(deps) = graph.deps.get(id) {
            for dep_id in deps {
                if graph.workspace_members.contains(dep_id) {
                    actual_edges.insert((from_name.clone(), graph.name_of(dep_id).to_string()));
                }
            }
        }
    }

    let required: HashSet<(String, String)> = args.required_edges.iter().cloned().collect();
    let missing: Vec<(String, String)> = required.difference(&actual_edges).cloned().collect();
    let extra: Vec<(String, String)> = if args.forbid_extra_edges {
        actual_edges.difference(&required).cloned().collect()
    } else {
        vec![]
    };

    // Bin target counts, from packages[].targets where kind == ["bin"].
    let mut bin_counts: BTreeMap<String, u32> = BTreeMap::new();
    if let Some(packages) = metadata.get("packages").and_then(Value::as_array) {
        for pkg in packages {
            let name = pkg.get("name").and_then(Value::as_str).unwrap_or_default();
            let targets = pkg
                .get("targets")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let count = targets
                .iter()
                .filter(|t| {
                    t.get("kind")
                        .and_then(Value::as_array)
                        .map(|k| k.iter().any(|x| x.as_str() == Some("bin")))
                        .unwrap_or(false)
                })
                .count() as u32;
            if count > 0 {
                *bin_counts.entry(name.to_string()).or_insert(0) += count;
            }
        }
    }

    let mut bin_mismatches: Vec<Value> = Vec::new();
    for (crate_name, expected_count) in &args.expected_bin_targets {
        let actual_count = bin_counts.get(crate_name).copied().unwrap_or(0);
        if actual_count != *expected_count {
            bin_mismatches.push(json!({
                "crate": crate_name,
                "expected": expected_count,
                "actual": actual_count,
            }));
        }
    }

    let mut missing_sorted = missing.clone();
    missing_sorted.sort();
    let mut extra_sorted = extra.clone();
    extra_sorted.sort();

    if missing_sorted.is_empty() && extra_sorted.is_empty() && bin_mismatches.is_empty() {
        (
            ObligationStatus::Passed,
            json!({ "edges_checked": required.len(), "bin_targets_checked": args.expected_bin_targets.len() }),
        )
    } else {
        (
            ObligationStatus::Failed,
            json!({
                "missing_edges": missing_sorted,
                "extra_edges": extra_sorted,
                "bin_mismatches": bin_mismatches,
            }),
        )
    }
}

// ── Shared: fetch + parse `cargo metadata`, build a reachability graph ──────

fn fetch_metadata(
    cargo_bin: &Path,
    workspace_root: &Path,
    build_config: &BuildConfig,
) -> Result<Value, String> {
    let mut args: Vec<&str> = vec!["metadata", "--format-version=1"];
    let joined_features;
    match build_config {
        BuildConfig::Default => {}
        BuildConfig::Features(features) => {
            joined_features = features.join(" ");
            args.push("--features");
            args.push(&joined_features);
        }
    }
    let captured = run_tool(cargo_bin, &args, Some(workspace_root)).map_err(|e| e.to_string())?;
    if captured.exit_code != 0 {
        return Err(format!(
            "cargo metadata exited {}: {}",
            captured.exit_code, captured.stderr
        ));
    }
    serde_json::from_str(&captured.stdout)
        .map_err(|e| format!("failed to parse cargo metadata JSON: {e}"))
}

struct Graph {
    /// package id -> crate name (as declared in that crate's own Cargo.toml).
    names: BTreeMap<String, String>,
    /// package id -> resolved (feature-respecting) dependency package ids,
    /// ALL dependency kinds (normal + build + dev). Used by `dep_graph`.
    deps: BTreeMap<String, Vec<String>>,
    /// package id -> resolved dependency package ids restricted to edges
    /// that ship (normal + build; dev-only edges dropped — Q4 ruling, see
    /// module docs). Used by `no_dep`.
    deps_shipped: BTreeMap<String, Vec<String>>,
    workspace_members: HashSet<String>,
}

impl Graph {
    fn from_metadata(metadata: &Value) -> Result<Self, String> {
        let mut names = BTreeMap::new();
        if let Some(packages) = metadata.get("packages").and_then(Value::as_array) {
            for pkg in packages {
                let id = pkg
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("package missing id")?;
                let name = pkg
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("package missing name")?;
                names.insert(id.to_string(), name.to_string());
            }
        }

        let mut deps = BTreeMap::new();
        let mut deps_shipped = BTreeMap::new();
        let nodes = metadata
            .get("resolve")
            .and_then(|r| r.get("nodes"))
            .and_then(Value::as_array)
            .ok_or("cargo metadata output missing resolve.nodes")?;
        for node in nodes {
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .ok_or("resolve node missing id")?;
            let node_deps: Vec<String> = node
                .get("dependencies")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            deps.insert(id.to_string(), node_deps);

            // Kind-scoped edges from `resolve.nodes[].deps[]`, whose
            // `dep_kinds[].kind` is null (normal), "build", or "dev".
            // An edge ships if ANY of its kinds is normal/build — a crate
            // that is both a dev-dep and a normal dep is still shipped.
            let shipped: Vec<String> = node
                .get("deps")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter(|d| {
                            d.get("dep_kinds")
                                .and_then(Value::as_array)
                                .map(|kinds| {
                                    kinds.is_empty()
                                        || kinds.iter().any(|k| {
                                            matches!(k.get("kind"), Some(Value::Null) | None)
                                                || k.get("kind").and_then(Value::as_str)
                                                    == Some("build")
                                        })
                                })
                                // Older cargo without dep_kinds: keep the
                                // edge (kind-blind fallback, never silently
                                // NARROWS scope).
                                .unwrap_or(true)
                        })
                        .filter_map(|d| d.get("pkg").and_then(Value::as_str).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            deps_shipped.insert(id.to_string(), shipped);
        }

        let workspace_members: HashSet<String> = metadata
            .get("workspace_members")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Graph {
            names,
            deps,
            deps_shipped,
            workspace_members,
        })
    }

    fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.names.get(id).map(String::as_str).unwrap_or(id)
    }

    /// Workspace-member package ids matching `scope_crates` by name; all
    /// workspace members if `scope_crates` is empty.
    fn resolve_scope_roots(&self, scope_crates: &[String]) -> Vec<String> {
        if scope_crates.is_empty() {
            return self.workspace_members.iter().cloned().collect();
        }
        self.workspace_members
            .iter()
            .filter(|id| scope_crates.iter().any(|c| self.name_of(id) == c))
            .cloned()
            .collect()
    }

    /// BFS from `roots` over the resolved dependency graph. Returns every
    /// reachable package id (roots' transitive deps; roots themselves are
    /// NOT included unless self-referential, which cargo metadata never
    /// produces). `shipped_only` restricts the walk to normal+build edges
    /// (Q4 ruling — the deny-list scope).
    fn reachable_from(&self, roots: &[String], shipped_only: bool) -> Vec<(String, String)> {
        let edges = if shipped_only {
            &self.deps_shipped
        } else {
            &self.deps
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = roots.iter().cloned().collect();
        let mut out = Vec::new();
        while let Some(id) = queue.pop_front() {
            let Some(children) = edges.get(&id) else {
                continue;
            };
            for child in children {
                if seen.insert(child.clone()) {
                    out.push((child.clone(), self.name_of(child).to_string()));
                    queue.push_back(child.clone());
                }
            }
        }
        out
    }

    /// First direct parent (by name) of `id` found while walking outward
    /// from `roots` — one-hop provenance for evidence messages (contract §5
    /// example: `"via": "nanoid"`). Not necessarily the ONLY path, just a
    /// deterministic, reproducible one (BFS order over a `BTreeMap`-backed
    /// graph is itself deterministic).
    fn first_parent_name(&self, target: &str, roots: &[String]) -> Option<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = roots.iter().cloned().collect();
        for r in roots {
            seen.insert(r.clone());
        }
        while let Some(id) = queue.pop_front() {
            // Same edge scope as the `no_dep` walk that produced the match.
            let Some(children) = self.deps_shipped.get(&id) else {
                continue;
            };
            for child in children {
                if child == target {
                    return Some(self.name_of(&id).to_string());
                }
                if seen.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BuildConfig;

    fn workspace_root() -> std::path::PathBuf {
        // crates/g8-obligations -> crates -> workspace root
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn no_dep_passes_when_denied_crate_absent() {
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Exact(
                "this-crate-does-not-exist-anywhere".to_string(),
            )],
            scope_crates: vec!["g8-core".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn no_dep_fails_when_denied_crate_present() {
        // serde is a real, always-present transitive dep of g8-core.
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Exact("serde".to_string())],
            scope_crates: vec!["g8-core".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert_eq!(detail["matched"][0], "serde");
    }

    #[test]
    fn no_dep_regex_matcher_works() {
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Regex("^ser".to_string())],
            scope_crates: vec!["g8-core".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, _) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed);
    }

    #[test]
    fn no_dep_excludes_dev_only_dependencies() {
        // Q4 ruling regression: `tempfile` is dev-deps-only in g8-store —
        // it must NOT count against the deny-list (dev edges don't ship).
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Exact("tempfile".to_string())],
            scope_crates: vec!["g8-store".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
        assert_eq!(detail["dependency_kinds_in_scope"][0], "normal");
    }

    #[test]
    fn no_dep_rand_chain_clean_in_deterministic_zone_post_q4() {
        // The exact D11-02 core, pinned as a regression test: after the
        // nanoid -> content-hash replacement, no rand-family crate is
        // reachable through any shipped edge of the six original crates.
        let args = CargoMetadataNoDepArgs {
            denied: vec![
                DepMatcher::Exact("getrandom".to_string()),
                DepMatcher::Exact("fastrand".to_string()),
                DepMatcher::Exact("rand".to_string()),
                DepMatcher::Exact("rand_core".to_string()),
                DepMatcher::Exact("nanoid".to_string()),
            ],
            scope_crates: vec![
                "g8-core".to_string(),
                "g8-extractor".to_string(),
                "g8-store".to_string(),
                "g8-planner".to_string(),
                "g8-conflict".to_string(),
                "g8".to_string(),
            ],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn no_dep_respects_default_feature_gating_for_serve() {
        // tiny_http is only pulled in by g8's optional `serve` feature.
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Exact("tiny_http".to_string())],
            scope_crates: vec!["g8".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn no_dep_features_build_config_finds_tiny_http() {
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Exact("tiny_http".to_string())],
            scope_crates: vec!["g8".to_string()],
            build_config: BuildConfig::Features(vec!["g8/serve".to_string()]),
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
    }

    #[test]
    fn no_dep_error_path_missing_cargo_binary() {
        let args = CargoMetadataNoDepArgs {
            denied: vec![],
            scope_crates: vec![],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep_with_binary(
            Path::new("/nonexistent/cargo-binary"),
            &args,
            &workspace_root(),
        );
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn no_dep_error_path_unknown_scope_crate() {
        let args = CargoMetadataNoDepArgs {
            denied: vec![],
            scope_crates: vec!["this-crate-is-not-in-the-workspace".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn no_dep_error_path_malformed_regex() {
        let args = CargoMetadataNoDepArgs {
            denied: vec![DepMatcher::Regex("(unclosed".to_string())],
            scope_crates: vec!["g8-core".to_string()],
            build_config: BuildConfig::Default,
        };
        let (status, detail) = no_dep(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Error, "detail: {detail}");
    }

    #[test]
    fn dep_graph_passes_on_real_required_edge() {
        let args = CargoMetadataDepGraphArgs {
            required_edges: vec![("g8-store".to_string(), "g8-core".to_string())],
            forbid_extra_edges: false,
            expected_bin_targets: vec![("g8".to_string(), 1)],
        };
        let (status, detail) = dep_graph(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Passed, "detail: {detail}");
    }

    #[test]
    fn dep_graph_fails_on_missing_required_edge() {
        let args = CargoMetadataDepGraphArgs {
            required_edges: vec![("g8-core".to_string(), "g8-store".to_string())], // wrong direction
            forbid_extra_edges: false,
            expected_bin_targets: vec![],
        };
        let (status, detail) = dep_graph(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
        assert!(!detail["missing_edges"].as_array().unwrap().is_empty());
    }

    #[test]
    fn dep_graph_fails_on_wrong_bin_count() {
        let args = CargoMetadataDepGraphArgs {
            required_edges: vec![],
            forbid_extra_edges: false,
            expected_bin_targets: vec![("g8".to_string(), 99)],
        };
        let (status, detail) = dep_graph(&args, &workspace_root());
        assert_eq!(status, ObligationStatus::Failed, "detail: {detail}");
    }

    #[test]
    fn dep_graph_error_path_missing_cargo_binary() {
        let args = CargoMetadataDepGraphArgs {
            required_edges: vec![],
            forbid_extra_edges: false,
            expected_bin_targets: vec![],
        };
        let (status, _) = dep_graph_with_binary(
            Path::new("/nonexistent/cargo-binary"),
            &args,
            &workspace_root(),
        );
        assert_eq!(status, ObligationStatus::Error);
    }
}
