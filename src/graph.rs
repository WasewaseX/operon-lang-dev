//! W094, static graph export for `regulate` networks.
//!
//! `operon graph f.op [--json]` walks the parsed program (no execution,
//! the same static stance as `check`/`fmt`), collects every `regulate`
//! edge from top-level statements plus gene bodies / frames / TAD blocks
//! (mechanisms may be declared inside any of those), and renders either
//! Graphviz DOT or a self-describing JSON document.
//!
//! Node/edge semantics per SPEC §11: an edge is `activates` (inhibit=false)
//! or `inhibits` (inhibit=true) with `strength` (default 1.0) and an
//! optional Hill-style dose `threshold`. Levels themselves are runtime
//! state, this export is structure only.

use crate::ast::{RegEdge, Stmt, TransEdge};

#[derive(Debug, Default, Clone)]
pub struct GraphDump {
    pub edges: Vec<RegEdge>,
    /// two-tier translation edges (reg-bio-2): mRNA -> protein with rate/decay
    pub trans: Vec<TransEdge>,
    pub nodes: Vec<String>,
}

impl GraphDump {
    fn push_node(&mut self, n: &str) {
        if !self.nodes.iter().any(|x| x == n) {
            self.nodes.push(n.to_string());
        }
    }
}

/// Collect all regulate edges reachable from the statement list.
pub fn collect(stmts: &[Stmt]) -> GraphDump {
    let mut g = GraphDump::default();
    walk(stmts, &mut g);
    g
}

fn walk(stmts: &[Stmt], g: &mut GraphDump) {
    for s in stmts {
        match s {
            Stmt::Regulate(es, trans, _binds) => {
                // v1 renders regulation + translation edges; BindDef records
                // (allostery) are node annotations, v2 scope (TODO-100 W094).
                for e in es {
                    g.push_node(&e.from);
                    g.push_node(&e.to);
                    g.edges.push(e.clone());
                }
                for t in trans {
                    g.push_node(&t.from);
                    g.push_node(&t.to);
                    g.trans.push(t.clone());
                }
            }
            // mechanisms may nest: descend into the containers
            Stmt::Gene(def) => walk(&def.body, g),
            Stmt::Frame { body, .. } => walk(body, g),
            Stmt::Tad(_, body) => walk(body, g),
            _ => {}
        }
    }
}

/// Graphviz DOT rendering. Inhibition edges are dashed crimson; labels carry
/// strength and (when declared) the dose threshold, the two knobs a reader
/// of SPEC §11 needs to reconstruct the dynamics.
pub fn to_dot(g: &GraphDump) -> String {
    let mut out = String::from("digraph operon {\n  rankdir=LR;\n");
    for n in &g.nodes {
        out.push_str(&format!("  \"{}\";\n", n));
    }
    for t in &g.trans {
        let rate = t.rate.map(fmt_num).unwrap_or_else(|| "1".to_string());
        out.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"translates x{}\", style=dotted];\n",
            t.from, t.to, rate
        ));
    }
    for e in &g.edges {
        let kind = if e.inhibit { "inhibits" } else { "activates" };
        let mut label = format!("{} x{}", kind, fmt_num(e.strength));
        let mut style = "";
        if e.inhibit {
            style = ", style=dashed, color=crimson";
        }
        if let Some(t) = e.threshold {
            label.push_str(&format!(" (t={})", fmt_num(t)));
        }
        out.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{}\"{}];\n",
            e.from, e.to, label, style
        ));
    }
    out.push_str("}\n");
    out
}

/// Self-describing JSON rendering (pairs with `operon profile --json`).
pub fn to_json(g: &GraphDump) -> String {
    let nodes: Vec<String> = g
        .nodes
        .iter()
        .map(|n| format!("\"{}\"", crate::tools::json_escape(n)))
        .collect();
    let edges: Vec<String> = g
        .edges
        .iter()
        .map(|e| {
            let thr = match e.threshold {
                Some(t) => format!("{:.1}", t),
                None => "null".to_string(),
            };
            format!(
                "{{\"from\":\"{}\",\"to\":\"{}\",\"strength\":{},\"inhibit\":{},\"threshold\":{}}}",
                crate::tools::json_escape(&e.from),
                crate::tools::json_escape(&e.to),
                fmt_num(e.strength),
                e.inhibit,
                thr
            )
        })
        .collect();
    let trans: Vec<String> = g
        .trans
        .iter()
        .map(|t| {
            let rate = match t.rate {
                Some(r) => format!("{:.1}", r),
                None => "null".to_string(),
            };
            let decay = match t.decay {
                Some(d) => format!("{:.1}", d),
                None => "null".to_string(),
            };
            format!(
                "{{\"from\":\"{}\",\"to\":\"{}\",\"rate\":{},\"decay\":{}}}",
                crate::tools::json_escape(&t.from),
                crate::tools::json_escape(&t.to),
                rate,
                decay
            )
        })
        .collect();
    format!(
        "{{\"format\":\"operon-graph\",\"version\":\"{}\",\"nodes\":[{}],\"edges\":[{}],\"trans_edges\":[{}]}}",
        env!("CARGO_PKG_VERSION"),
        nodes.join(","),
        edges.join(","),
        trans.join(",")
    )
}

/// Trim trailing zeros so 1.0 renders as "1", 0.8 as "0.8" (stable, and
/// pleasant in DOT labels).
fn fmt_num(v: f64) -> String {
    let s = format!("{}", v);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_and_renders() {
        let src = "regulate { a activates b strength 0.8; c inhibits d threshold 0.3; }\n";
        let prog = crate::parser::parse(src);
        let g = collect(&prog.stmts);
        assert_eq!(g.nodes.len(), 4, "a b c d");
        assert_eq!(g.edges.len(), 2);
        assert!(!g.edges[0].inhibit);
        assert!(g.edges[1].inhibit);
        assert_eq!(g.edges[1].threshold, Some(0.3));
        let dot = to_dot(&g);
        assert!(dot.starts_with("digraph operon {"));
        assert!(dot.contains("\"a\" -> \"b\" [label=\"activates x0.8\"]"));
        assert!(dot.contains("style=dashed"));
        assert!(dot.contains("(t=0.3)"));
        let js = to_json(&g);
        assert!(js.contains("\"format\":\"operon-graph\""));
        assert!(js.contains("\"threshold\":0.3"));
        assert!(js.contains("\"inhibit\":true"));
    }

    #[test]
    fn translation_edges_render() {
        let src = "regulate { mrna translates prot rate 0.5 }\n";
        let prog = crate::parser::parse(src);
        let g = collect(&prog.stmts);
        assert_eq!(g.trans.len(), 1, "trans edge collected");
        assert_eq!(g.edges.len(), 0, "reg edges untouched");
        assert!(to_dot(&g).contains("translates x0.5"));
        assert!(to_json(&g).contains("\"trans_edges\":[{\"from\":\"mrna\""));
    }

    #[test]
    fn nested_and_dedup() {
        let src = "gene wrap() {\n  regulate { p activates q }\n}\nframe meta {\n  regulate { q inhibits r strength 2 }\n}\nregulate { p activates q }\n";
        let prog = crate::parser::parse(src);
        let g = collect(&prog.stmts);
        assert_eq!(g.edges.len(), 3, "all three nets found (nested + top)");
        assert_eq!(g.nodes.len(), 3, "p q r deduped");
    }
}
