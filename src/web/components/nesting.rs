//! SPEC-048 REQ-4807 — Bounded, acyclic nesting.
//!
//! (a) Static cycle detection over the component-invocation graph (a component
//! template's `{% component %}` sites are statically discoverable) — a cycle is a
//! compile-time `component-cycle` naming the path. (b) A render-time depth cap
//! (NFR-4803, default 16) — a breach is `component-depth-bound`.

use super::{CResult, ComponentError};
use std::collections::{BTreeMap, BTreeSet};

/// NFR-4803 fail-closed default: maximum component nest depth.
pub const MAX_NEST_DEPTH: usize = 16;

/// The static component-invocation graph: component name → the components it invokes.
pub type InvocationGraph = BTreeMap<String, BTreeSet<String>>;

/// Detect a cycle in the component-invocation graph. On a cycle, returns
/// `component-cycle` whose message names the cycle path (e.g. `a -> b -> a`).
pub fn detect_cycle(graph: &InvocationGraph) -> CResult<()> {
    // DFS with a recursion stack; deterministic via BTree ordering.
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut on_stack: Vec<String> = Vec::new();

    fn visit(
        node: &str,
        graph: &InvocationGraph,
        visited: &mut BTreeSet<String>,
        on_stack: &mut Vec<String>,
    ) -> CResult<()> {
        if let Some(pos) = on_stack.iter().position(|n| n == node) {
            let mut path = on_stack[pos..].to_vec();
            path.push(node.to_string());
            return Err(ComponentError::new(
                "component-cycle",
                format!("component cycle: {}", path.join(" -> ")),
            ));
        }
        if visited.contains(node) {
            return Ok(());
        }
        on_stack.push(node.to_string());
        if let Some(edges) = graph.get(node) {
            for next in edges {
                visit(next, graph, visited, on_stack)?;
            }
        }
        on_stack.pop();
        visited.insert(node.to_string());
        Ok(())
    }

    for node in graph.keys() {
        visit(node, graph, &mut visited, &mut on_stack)?;
    }
    Ok(())
}

/// Render-time depth guard. Returns `component-depth-bound` when `depth` exceeds
/// [`MAX_NEST_DEPTH`].
pub fn check_depth(depth: usize) -> CResult<()> {
    if depth > MAX_NEST_DEPTH {
        Err(ComponentError::new(
            "component-depth-bound",
            format!("component/transclusion nest depth {depth} exceeds cap {MAX_NEST_DEPTH}"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> InvocationGraph {
        edges
            .iter()
            .map(|(k, vs)| {
                (
                    k.to_string(),
                    vs.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
                )
            })
            .collect()
    }

    #[test]
    fn acyclic_graph_ok() {
        let g = graph(&[("a", &["b", "c"]), ("b", &["c"]), ("c", &[])]);
        assert!(detect_cycle(&g).is_ok());
    }

    #[test]
    fn direct_cycle_detected() {
        let g = graph(&[("a", &["a"])]);
        let err = detect_cycle(&g).unwrap_err();
        assert_eq!(err.code, "component-cycle");
        assert!(err.message.contains("a -> a"));
    }

    #[test]
    fn mutual_cycle_named() {
        let g = graph(&[("a", &["b"]), ("b", &["a"])]);
        let err = detect_cycle(&g).unwrap_err();
        assert_eq!(err.code, "component-cycle");
        assert!(err.message.contains("a -> b -> a") || err.message.contains("b -> a -> b"));
    }

    #[test]
    fn depth_bound_enforced() {
        assert!(check_depth(MAX_NEST_DEPTH).is_ok());
        assert_eq!(
            check_depth(MAX_NEST_DEPTH + 1).unwrap_err().code,
            "component-depth-bound"
        );
    }
}
