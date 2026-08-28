//! Serialize a [`Graph`] into an AgentViz **scene** JSON document.
//!
//! AgentViz (`~/dev/agentviz`) renders node-edge scenes in a WebGPU 3D
//! explorer. Its `SceneSpec` deserializer (`src/scene/types.rs`) accepts a
//! `graph_explorer` scene with per-node `color`/`shape`/`group` and directed
//! edges. We emit explicit colors per G8 entity kind (and per plan status) so
//! the convergence space is legible without relying on AgentViz's own palette.
//!
//! Load it in the explorer with:
//!   http://localhost:8090/?scene=<url-to-/api/scene>
//! or save the file and `agentviz scene load <file>`.

use super::graph::Graph;
use serde_json::{json, Value};

/// RGBA in 0.0–1.0, matching `NodeSpec.color`.
type Rgba = [f32; 4];

fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// Color a node by kind, falling back to status for plans.
fn node_color(kind: &str, status: Option<&str>) -> Rgba {
    match kind {
        "plan" => match status.unwrap_or("Idea") {
            "Idea" => rgb(0x62, 0x72, 0xa4),
            "Scoped" => rgb(0x8b, 0xe9, 0xfd),
            "Dispatched" => rgb(0x50, 0xfa, 0x7b),
            "Blocked" => rgb(0xff, 0x55, 0x55),
            "Done" => rgb(0x44, 0x47, 0x5a),
            "Parked" => rgb(0xff, 0xb8, 0x6c),
            _ => rgb(0x62, 0x72, 0xa4),
        },
        "capability" => rgb(0x2d, 0xd4, 0xbf),
        "intent" => rgb(0xbd, 0x93, 0xf9),
        "decision" => rgb(0xff, 0xb8, 0x6c),
        "substrate" => rgb(0xf8, 0xf8, 0xf2),
        "project" => rgb(0x7f, 0x8e, 0xa3),
        _ => rgb(0x8b, 0x91, 0xa1),
    }
}

/// AgentViz shape glyph name per kind (see `src/scene/convert.rs` SHAPE_NAMES).
fn node_shape(kind: &str) -> &'static str {
    match kind {
        "plan" => "circle",
        "capability" => "diamond",
        "intent" => "triangle-up",
        "decision" => "square",
        "substrate" => "hexagon",
        "project" => "rounded-square",
        _ => "circle",
    }
}

/// Stable numeric category index per kind (drives clustering/legend).
fn node_category(kind: &str) -> u32 {
    match kind {
        "plan" => 0,
        "capability" => 1,
        "intent" => 2,
        "decision" => 3,
        "substrate" => 4,
        "project" => 5,
        _ => 6,
    }
}

/// Relative node size; substrates are the structural hubs, dispatched plans pop.
fn node_size(kind: &str, status: Option<&str>) -> f32 {
    match kind {
        "substrate" => 1.8,
        "project" => 1.4,
        "plan" => match status {
            Some("Dispatched") | Some("Blocked") => 1.2,
            _ => 1.0,
        },
        _ => 0.9,
    }
}

/// Edge attraction strength; structural edges (`on`, `in`) pull harder so
/// substrates/projects act as gravity wells in the force layout.
fn edge_strength(kind: &str) -> f32 {
    match kind {
        "on" | "in" => 0.8,
        "conflicts" | "contradicts" | "violates" => 0.3,
        _ => 0.5,
    }
}

/// Build the AgentViz scene document from a convergence graph.
pub fn to_scene(graph: &Graph) -> Value {
    let nodes: Vec<Value> = graph
        .nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.id,
                "label": n.label,
                "color": node_color(&n.kind, n.status.as_deref()),
                "shape": node_shape(&n.kind),
                "category": node_category(&n.kind),
                "group": n.kind,
                "size": node_size(&n.kind, n.status.as_deref()),
                "importance": node_size(&n.kind, n.status.as_deref()).min(1.0),
                "kind_label": n.kind,
                "status": n.status,
                "substrate": n.substrate,
            })
        })
        .collect();

    let edges: Vec<Value> = graph
        .edges
        .iter()
        .map(|e| {
            json!({
                "source": e.source,
                "target": e.target,
                "weight": 1.0,
                "edge_type": 0,
                "directed": true,
                "strength": edge_strength(&e.kind),
                "kind": e.kind,
            })
        })
        .collect();

    json!({
        "version": 1,
        "mode": "graph_explorer",
        "name": format!("g8 · {}", graph.space.name),
        "nodes": nodes,
        "edges": edges,
        "style": { "background": [0.055, 0.06, 0.07], "edge_alpha": 0.35 },
        "layout": { "type": "force", "running": true },
    })
}
