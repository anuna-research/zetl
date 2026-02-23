use crate::types::ParsedFile;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

/// The link graph built from parsed files
pub struct LinkGraph {
    pub graph: DiGraph<String, EdgeMeta>,
    pub node_map: HashMap<String, NodeIndex>,
    /// Set of page names that correspond to real files (not phantom/unresolved targets)
    pub resolved: HashSet<String>,
}

/// Metadata on a graph edge (link)
#[derive(Debug, Clone, Serialize)]
pub struct EdgeMeta {
    pub source_file: String,
    pub line: u32,
    pub alias: Option<String>,
    pub heading: Option<String>,
    pub block_ref: Option<String>,
    pub is_embed: bool,
}

#[derive(Debug, Clone)]
pub struct ForwardLinkResult {
    pub target: String,
    pub meta: EdgeMeta,
}

#[derive(Debug, Serialize)]
pub struct BacklinkResult {
    pub source: String,
    pub line: u32,
    pub context: Option<String>,
    pub alias: Option<String>,
    pub is_embed: bool,
}

#[derive(Debug, Serialize)]
pub struct DeadLink {
    pub source: String,
    pub line: u32,
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct Orphan {
    pub page: String,
    pub forward_links: usize,
}

#[derive(Debug, Serialize)]
pub struct PathResult {
    pub from: String,
    pub to: String,
    pub hops: usize,
    pub path: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GraphStats {
    pub pages: usize,
    pub links: usize,
    pub unique_targets: usize,
    pub dead_links: usize,
    pub orphans: usize,
    pub ambiguous_links: usize,
    pub connected_components: usize,
    pub most_linked: Vec<MostLinked>,
}

#[derive(Debug, Serialize)]
pub struct MostLinked {
    pub page: String,
    pub backlink_count: usize,
}

impl LinkGraph {
    /// Build the link graph from parsed files.
    ///
    /// - `files`: parsed markdown files from the vault
    /// - `resolved_pages`: maps raw link targets (lowercased/normalized) to resolved page names
    ///
    /// Creates a node for every real page (from files) and a phantom node for every
    /// unresolved link target. Edges carry `EdgeMeta` with source file info.
    pub fn build(files: &[ParsedFile], resolved_pages: &HashMap<String, String>) -> Self {
        let mut graph = DiGraph::<String, EdgeMeta>::new();
        let mut node_map: HashMap<String, NodeIndex> = HashMap::new();
        let mut resolved: HashSet<String> = HashSet::new();

        // 1. Create a node for each file's page_name (these are "real" nodes)
        for file in files {
            let name = file.page_name.clone();
            if !node_map.contains_key(&name) {
                let idx = graph.add_node(name.clone());
                node_map.insert(name.clone(), idx);
            }
            resolved.insert(name);
        }

        // 2. For each link in each file, resolve the target and create edges
        for file in files {
            let source_name = &file.page_name;
            let source_idx = node_map[source_name];

            for link in &file.links {
                // Try to resolve via the resolved_pages map; fall back to raw_target
                let target_name = resolved_pages
                    .get(&link.raw_target)
                    .cloned()
                    .unwrap_or_else(|| link.target_page.clone());

                // Create the target node if it doesn't exist yet (phantom node)
                let target_idx = *node_map
                    .entry(target_name.clone())
                    .or_insert_with(|| graph.add_node(target_name.clone()));

                let meta = EdgeMeta {
                    source_file: file.path.to_string_lossy().to_string(),
                    line: link.line,
                    alias: link.alias.clone(),
                    heading: link.heading.clone(),
                    block_ref: link.block_ref.clone(),
                    is_embed: link.is_embed,
                };

                graph.add_edge(source_idx, target_idx, meta);
            }
        }

        LinkGraph {
            graph,
            node_map,
            resolved,
        }
    }

    /// Get forward links (outgoing edges) from a page.
    pub fn forward_links(&self, page: &str) -> Vec<ForwardLinkResult> {
        let Some(&idx) = self.node_map.get(page) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|edge| ForwardLinkResult {
                target: self.graph[edge.target()].clone(),
                meta: edge.weight().clone(),
            })
            .collect()
    }

    /// Get backlinks (incoming edges) to a page.
    pub fn backlinks(&self, page: &str) -> Vec<BacklinkResult> {
        let Some(&idx) = self.node_map.get(page) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|edge| {
                let meta = edge.weight();
                let source_node = edge.source();
                let source_name = self.graph[source_node].clone();
                BacklinkResult {
                    source: source_name,
                    line: meta.line,
                    context: None,
                    alias: meta.alias.clone(),
                    is_embed: meta.is_embed,
                }
            })
            .collect()
    }

    /// Find dead links: edges whose target is a phantom node (not backed by a real file).
    pub fn dead_links(&self) -> Vec<DeadLink> {
        let mut result = Vec::new();

        for edge_ref in self.graph.edge_references() {
            let target_idx = edge_ref.target();
            let target_name = &self.graph[target_idx];

            if !self.resolved.contains(target_name) {
                let source_idx = edge_ref.source();
                let source_name = &self.graph[source_idx];
                let meta = edge_ref.weight();

                result.push(DeadLink {
                    source: source_name.clone(),
                    line: meta.line,
                    target: target_name.clone(),
                });
            }
        }

        result
    }

    /// Find orphan pages: real nodes with zero incoming edges.
    /// Returns each orphan with its count of forward (outgoing) links.
    pub fn orphans(&self) -> Vec<Orphan> {
        let mut result = Vec::new();

        for page_name in &self.resolved {
            let &idx = &self.node_map[page_name];
            let incoming_count = self.graph.edges_directed(idx, Direction::Incoming).count();

            if incoming_count == 0 {
                let forward_count = self.graph.edges_directed(idx, Direction::Outgoing).count();

                result.push(Orphan {
                    page: page_name.clone(),
                    forward_links: forward_count,
                });
            }
        }

        // Sort by page name for deterministic output
        result.sort_by(|a, b| a.page.cmp(&b.page));
        result
    }

    /// Find shortest path between two pages using BFS, limited to `max_depth` hops.
    pub fn shortest_path(&self, from: &str, to: &str, max_depth: usize) -> Option<PathResult> {
        let &start = self.node_map.get(from)?;
        let &end = self.node_map.get(to)?;

        if start == end {
            return Some(PathResult {
                from: from.to_string(),
                to: to.to_string(),
                hops: 0,
                path: vec![from.to_string()],
            });
        }

        // BFS with depth tracking
        let mut visited: HashMap<NodeIndex, NodeIndex> = HashMap::new(); // child -> parent
        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();

        visited.insert(start, start); // sentinel: start's parent is itself
        queue.push_back((start, 0));

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            for neighbor in self.graph.neighbors_directed(current, Direction::Outgoing) {
                if visited.contains_key(&neighbor) {
                    continue;
                }

                visited.insert(neighbor, current);

                if neighbor == end {
                    // Reconstruct path
                    let mut path = Vec::new();
                    let mut node = end;
                    loop {
                        path.push(self.graph[node].clone());
                        let parent = visited[&node];
                        if parent == node {
                            // We've reached start
                            break;
                        }
                        node = parent;
                    }
                    path.reverse();

                    let hops = path.len() - 1;
                    return Some(PathResult {
                        from: from.to_string(),
                        to: to.to_string(),
                        hops,
                        path,
                    });
                }

                queue.push_back((neighbor, depth + 1));
            }
        }

        None
    }

    /// Compute graph statistics.
    ///
    /// - `top_n`: how many entries to include in the `most_linked` list.
    pub fn stats(&self, top_n: usize) -> GraphStats {
        let pages = self.resolved.len();
        let links = self.graph.edge_count();

        // Unique targets: count distinct target nodes across all edges
        let unique_targets: HashSet<NodeIndex> =
            self.graph.edge_references().map(|e| e.target()).collect();
        let unique_targets_count = unique_targets.len();

        let dead_links = self.dead_links().len();
        let orphans = self.orphans().len();

        // Ambiguous links: not directly tracked in this graph, default to 0
        // (Would require the resolver to report ambiguities separately)
        let ambiguous_links = 0;

        // Connected components using petgraph's algo (treats the graph as undirected)
        let connected_components = petgraph::algo::connected_components(&self.graph);

        // Most linked: count incoming edges for each real node, sort descending
        let mut backlink_counts: Vec<MostLinked> = self
            .resolved
            .iter()
            .map(|name| {
                let &idx = &self.node_map[name];
                let count = self.graph.edges_directed(idx, Direction::Incoming).count();
                MostLinked {
                    page: name.clone(),
                    backlink_count: count,
                }
            })
            .collect();

        backlink_counts.sort_by(|a, b| {
            b.backlink_count
                .cmp(&a.backlink_count)
                .then_with(|| a.page.cmp(&b.page))
        });

        backlink_counts.truncate(top_n);

        GraphStats {
            pages,
            links,
            unique_targets: unique_targets_count,
            dead_links,
            orphans,
            ambiguous_links,
            connected_components,
            most_linked: backlink_counts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ParsedFile;
    use std::path::PathBuf;
    use std::time::SystemTime;

    /// Helper: build a minimal ParsedFile
    fn make_file(name: &str, links: Vec<(&str, u32)>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(format!("{name}.md")),
            page_name: name.to_string(),
            links: links
                .into_iter()
                .map(|(target, line)| crate::types::WikiLink {
                    target_page: target.to_string(),
                    raw_target: target.to_string(),
                    heading: None,
                    block_ref: None,
                    alias: None,
                    is_embed: false,
                    line,
                    column: 1,
                })
                .collect(),
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
        }
    }

    /// Helper: build a WikiLink with all options
    fn make_link(
        target: &str,
        line: u32,
        alias: Option<&str>,
        heading: Option<&str>,
        block_ref: Option<&str>,
        is_embed: bool,
    ) -> crate::types::WikiLink {
        crate::types::WikiLink {
            target_page: target.to_string(),
            raw_target: target.to_string(),
            heading: heading.map(|s| s.to_string()),
            block_ref: block_ref.map(|s| s.to_string()),
            alias: alias.map(|s| s.to_string()),
            is_embed,
            line,
            column: 1,
        }
    }

    fn simple_graph() -> LinkGraph {
        // A -> B, A -> C, B -> C, C -> A
        let files = vec![
            make_file("A", vec![("B", 1), ("C", 2)]),
            make_file("B", vec![("C", 1)]),
            make_file("C", vec![("A", 1)]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();

        LinkGraph::build(&files, &resolved)
    }

    #[test]
    fn test_build_creates_nodes_for_all_files() {
        let graph = simple_graph();
        assert_eq!(graph.node_map.len(), 3);
        assert!(graph.node_map.contains_key("A"));
        assert!(graph.node_map.contains_key("B"));
        assert!(graph.node_map.contains_key("C"));
    }

    #[test]
    fn test_build_creates_edges() {
        let graph = simple_graph();
        // A->B, A->C, B->C, C->A = 4 edges
        assert_eq!(graph.graph.edge_count(), 4);
    }

    #[test]
    fn test_build_resolved_set() {
        let graph = simple_graph();
        assert_eq!(graph.resolved.len(), 3);
        assert!(graph.resolved.contains("A"));
        assert!(graph.resolved.contains("B"));
        assert!(graph.resolved.contains("C"));
    }

    #[test]
    fn test_build_phantom_nodes() {
        // A links to "Ghost" which has no file
        let files = vec![make_file("A", vec![("Ghost", 1)])];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        assert_eq!(graph.node_map.len(), 2);
        assert!(graph.node_map.contains_key("A"));
        assert!(graph.node_map.contains_key("Ghost"));
        assert!(graph.resolved.contains("A"));
        assert!(!graph.resolved.contains("Ghost"));
    }

    #[test]
    fn test_build_with_resolved_pages_mapping() {
        // raw_target "note" resolves to "My Note"
        let files = [ParsedFile {
            path: PathBuf::from("daily.md"),
            page_name: "daily".to_string(),
            links: vec![make_link("note", 5, None, None, None, false)],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
        }];
        let mut resolved_pages = HashMap::new();
        resolved_pages.insert("note".to_string(), "My Note".to_string());

        // Also add "My Note" as a real file
        let files_with_target = vec![
            files[0].clone(),
            ParsedFile {
                path: PathBuf::from("My Note.md"),
                page_name: "My Note".to_string(),
                links: vec![],
                spl_blocks: vec![],
                diagnostics: vec![],
                mtime: SystemTime::now(),
            },
        ];

        let graph = LinkGraph::build(&files_with_target, &resolved_pages);
        // Edge should point to "My Note", not "note"
        let fwd = graph.forward_links("daily");
        assert_eq!(fwd.len(), 1);
        let target_idx = graph.node_map["My Note"];
        let edges: Vec<_> = graph
            .graph
            .edges_directed(graph.node_map["daily"], Direction::Outgoing)
            .collect();
        assert_eq!(edges[0].target(), target_idx);
    }

    #[test]
    fn test_forward_links() {
        let graph = simple_graph();
        let fwd = graph.forward_links("A");
        assert_eq!(fwd.len(), 2);
        // Verify lines
        let mut lines: Vec<u32> = fwd.iter().map(|e| e.meta.line).collect();
        lines.sort();
        assert_eq!(lines, vec![1, 2]);
    }

    #[test]
    fn test_forward_links_nonexistent_page() {
        let graph = simple_graph();
        let fwd = graph.forward_links("NonExistent");
        assert!(fwd.is_empty());
    }

    #[test]
    fn test_backlinks() {
        let graph = simple_graph();
        // C is linked from A (line 2) and B (line 1)
        let bl = graph.backlinks("C");
        assert_eq!(bl.len(), 2);
        let mut sources: Vec<String> = bl.iter().map(|b| b.source.clone()).collect();
        sources.sort();
        assert_eq!(sources, vec!["A", "B"]);
    }

    #[test]
    fn test_backlinks_nonexistent_page() {
        let graph = simple_graph();
        let bl = graph.backlinks("Ghost");
        assert!(bl.is_empty());
    }

    #[test]
    fn test_backlinks_with_alias_and_embed() {
        let files = [ParsedFile {
            path: PathBuf::from("source.md"),
            page_name: "source".to_string(),
            links: vec![make_link("target", 3, Some("display"), None, None, true)],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
        }];
        let mut resolved = HashMap::new();
        resolved.insert("target".to_string(), "target".to_string());

        let target_file = ParsedFile {
            path: PathBuf::from("target.md"),
            page_name: "target".to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
        };

        let graph = LinkGraph::build(&[files[0].clone(), target_file], &resolved);
        let bl = graph.backlinks("target");
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].source, "source");
        assert_eq!(bl[0].alias.as_deref(), Some("display"));
        assert!(bl[0].is_embed);
        assert_eq!(bl[0].line, 3);
    }

    #[test]
    fn test_dead_links() {
        let files = vec![
            make_file("A", vec![("B", 1), ("Ghost", 2)]),
            make_file("B", vec![("Phantom", 3)]),
        ];
        let mut resolved_pages = HashMap::new();
        resolved_pages.insert("A".to_string(), "A".to_string());
        resolved_pages.insert("B".to_string(), "B".to_string());

        let graph = LinkGraph::build(&files, &resolved_pages);
        let dead = graph.dead_links();

        assert_eq!(dead.len(), 2);
        let mut targets: Vec<String> = dead.iter().map(|d| d.target.clone()).collect();
        targets.sort();
        assert_eq!(targets, vec!["Ghost", "Phantom"]);
    }

    #[test]
    fn test_dead_links_none_when_all_resolved() {
        let graph = simple_graph();
        let dead = graph.dead_links();
        assert!(dead.is_empty());
    }

    #[test]
    fn test_orphans() {
        // D has no incoming links
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![("A", 1)]),
            make_file("D", vec![("A", 1)]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("D".to_string(), "D".to_string()),
        ]
        .into_iter()
        .collect();

        let graph = LinkGraph::build(&files, &resolved);
        let orphans = graph.orphans();

        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].page, "D");
        assert_eq!(orphans[0].forward_links, 1);
    }

    #[test]
    fn test_orphans_all_connected() {
        // A -> B -> C -> A: no orphans
        let graph = simple_graph();
        let orphans = graph.orphans();
        assert!(orphans.is_empty());
    }

    #[test]
    fn test_shortest_path_direct() {
        let graph = simple_graph();
        let result = graph.shortest_path("A", "B", 10).unwrap();
        assert_eq!(result.from, "A");
        assert_eq!(result.to, "B");
        assert_eq!(result.hops, 1);
        assert_eq!(result.path, vec!["A", "B"]);
    }

    #[test]
    fn test_shortest_path_multi_hop() {
        // A -> B -> C, no direct A -> C edge in this graph
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![("C", 1)]),
            make_file("C", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        let result = graph.shortest_path("A", "C", 10).unwrap();
        assert_eq!(result.hops, 2);
        assert_eq!(result.path, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_shortest_path_self() {
        let graph = simple_graph();
        let result = graph.shortest_path("A", "A", 10).unwrap();
        assert_eq!(result.hops, 0);
        assert_eq!(result.path, vec!["A"]);
    }

    #[test]
    fn test_shortest_path_no_path() {
        // A -> B, C is isolated
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![]),
            make_file("C", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        let result = graph.shortest_path("A", "C", 10);
        assert!(result.is_none());
    }

    #[test]
    fn test_shortest_path_max_depth_exceeded() {
        // A -> B -> C -> D, but max_depth = 2 means we can reach C but not D
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![("C", 1)]),
            make_file("C", vec![("D", 1)]),
            make_file("D", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
            ("D".to_string(), "D".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        // max_depth=2 allows paths of length <= 2 hops
        let result = graph.shortest_path("A", "D", 2);
        assert!(result.is_none());

        // max_depth=3 allows it
        let result = graph.shortest_path("A", "D", 3).unwrap();
        assert_eq!(result.hops, 3);
        assert_eq!(result.path, vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn test_shortest_path_nonexistent_node() {
        let graph = simple_graph();
        assert!(graph.shortest_path("A", "ZZZ", 10).is_none());
        assert!(graph.shortest_path("ZZZ", "A", 10).is_none());
    }

    #[test]
    fn test_stats_basic() {
        let graph = simple_graph();
        let stats = graph.stats(10);

        assert_eq!(stats.pages, 3);
        assert_eq!(stats.links, 4); // A->B, A->C, B->C, C->A
        assert_eq!(stats.dead_links, 0);
        assert_eq!(stats.orphans, 0);
        assert_eq!(stats.connected_components, 1);
    }

    #[test]
    fn test_stats_with_dead_links() {
        let files = vec![
            make_file("A", vec![("B", 1), ("Ghost", 2)]),
            make_file("B", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);
        let stats = graph.stats(10);

        assert_eq!(stats.pages, 2);
        assert_eq!(stats.links, 2); // A->B, A->Ghost
        assert_eq!(stats.dead_links, 1);
    }

    #[test]
    fn test_stats_connected_components() {
        // Two disconnected components: {A, B} and {C, D}
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![]),
            make_file("C", vec![("D", 1)]),
            make_file("D", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
            ("D".to_string(), "D".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);
        let stats = graph.stats(10);

        assert_eq!(stats.connected_components, 2);
    }

    #[test]
    fn test_stats_most_linked() {
        // C has 2 incoming (from A and B), B has 1 (from A), A has 0
        let files = vec![
            make_file("A", vec![("B", 1), ("C", 2)]),
            make_file("B", vec![("C", 1)]),
            make_file("C", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);
        let stats = graph.stats(2);

        assert_eq!(stats.most_linked.len(), 2);
        assert_eq!(stats.most_linked[0].page, "C");
        assert_eq!(stats.most_linked[0].backlink_count, 2);
        assert_eq!(stats.most_linked[1].page, "B");
        assert_eq!(stats.most_linked[1].backlink_count, 1);
    }

    #[test]
    fn test_stats_most_linked_top_n_truncation() {
        let files = vec![
            make_file("A", vec![("B", 1), ("C", 2), ("D", 3)]),
            make_file("B", vec![("C", 1), ("D", 2)]),
            make_file("C", vec![("D", 1)]),
            make_file("D", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
            ("D".to_string(), "D".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);
        let stats = graph.stats(1);

        assert_eq!(stats.most_linked.len(), 1);
        assert_eq!(stats.most_linked[0].page, "D");
        assert_eq!(stats.most_linked[0].backlink_count, 3);
    }

    #[test]
    fn test_unique_targets() {
        // A links to B twice, C once => unique targets = 2 (B and C)
        let files = vec![
            make_file("A", vec![("B", 1), ("B", 3), ("C", 5)]),
            make_file("B", vec![]),
            make_file("C", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);
        let stats = graph.stats(10);

        assert_eq!(stats.unique_targets, 2);
        assert_eq!(stats.links, 3);
    }

    #[test]
    fn test_empty_graph() {
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        assert!(graph.node_map.is_empty());
        assert!(graph.resolved.is_empty());
        assert!(graph.forward_links("anything").is_empty());
        assert!(graph.backlinks("anything").is_empty());
        assert!(graph.dead_links().is_empty());
        assert!(graph.orphans().is_empty());
        assert!(graph.shortest_path("a", "b", 10).is_none());

        let stats = graph.stats(10);
        assert_eq!(stats.pages, 0);
        assert_eq!(stats.links, 0);
        assert_eq!(stats.connected_components, 0);
    }

    #[test]
    fn test_self_link() {
        // A links to itself
        let files = vec![make_file("A", vec![("A", 1)])];
        let resolved: HashMap<String, String> =
            [("A".to_string(), "A".to_string())].into_iter().collect();
        let graph = LinkGraph::build(&files, &resolved);

        assert_eq!(graph.forward_links("A").len(), 1);
        assert_eq!(graph.backlinks("A").len(), 1);
        assert!(graph.dead_links().is_empty());
        // Self-linking node is not an orphan (has incoming edge from itself)
        assert!(graph.orphans().is_empty());
    }

    #[test]
    fn test_edge_meta_preserves_heading_and_block_ref() {
        let files = [ParsedFile {
            path: PathBuf::from("source.md"),
            page_name: "source".to_string(),
            links: vec![make_link(
                "target",
                7,
                Some("alias text"),
                Some("my-heading"),
                Some("block123"),
                false,
            )],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
        }];
        let target_file = ParsedFile {
            path: PathBuf::from("target.md"),
            page_name: "target".to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
        };
        let resolved: HashMap<String, String> = [("target".to_string(), "target".to_string())]
            .into_iter()
            .collect();

        let graph = LinkGraph::build(&[files[0].clone(), target_file], &resolved);
        let fwd = graph.forward_links("source");
        assert_eq!(fwd.len(), 1);
        assert_eq!(fwd[0].meta.heading.as_deref(), Some("my-heading"));
        assert_eq!(fwd[0].meta.block_ref.as_deref(), Some("block123"));
        assert_eq!(fwd[0].meta.alias.as_deref(), Some("alias text"));
        assert_eq!(fwd[0].meta.line, 7);
        assert_eq!(fwd[0].meta.source_file, "source.md");
        assert!(!fwd[0].meta.is_embed);
    }
}
