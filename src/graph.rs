use crate::types::ParsedFile;
use anyhow::{anyhow, Result};
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
    /// SPEC-045 named-edge predicate for this directed edge. `None` ⇒ an
    /// untyped edge (the pre-SPEC-045 default). A K-predicate wikilink expands
    /// to K edges, each carrying exactly one predicate (REQ-4506).
    #[serde(default)]
    pub predicate: Option<String>,
    /// SPEC-045 edge annotation (REQ-4504) — the indented sub-content under a
    /// list-item named edge, captured at the graph layer for progressive
    /// disclosure. `None` when absent.
    #[serde(default)]
    pub annotation: Option<String>,
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
    /// SPEC-045 predicate of the incoming edge (`None` ⇒ untyped).
    #[serde(default)]
    pub predicate: Option<String>,
    /// SPEC-045 annotation of the incoming edge (`None` ⇒ absent).
    #[serde(default)]
    pub annotation: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeadLink {
    pub source: String,
    pub line: u32,
    pub target: String,
    /// SPEC-045 predicate of the dead typed edge (`None` ⇒ untyped) — lets a
    /// curator see "3 dead `supersedes::` targets" (REQ-4506 / REQ-4514).
    #[serde(default)]
    pub predicate: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Orphan {
    pub page: String,
    pub forward_links: usize,
}

/// A single directed edge of the typed graph, flattened for the `zetl edges`
/// query surface (SPEC-045 REQ-4509 / CON-4503).
#[derive(Debug, Clone, Serialize)]
pub struct EdgeRecord {
    pub source: String,
    pub target: String,
    /// `None` ⇒ untyped edge.
    pub predicate: Option<String>,
    /// `None` ⇒ no annotation.
    pub annotation: Option<String>,
    pub line: u32,
    /// `true` when the target does not resolve to a real vault page (a typed
    /// or untyped ghost edge, REQ-4514).
    pub is_ghost: bool,
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

/// A node in a neighbourhood subgraph.
#[derive(Debug, Clone, Serialize)]
pub struct SubgraphNode {
    /// Page name (slug).
    pub slug: String,
    /// BFS depth from the root node (0 = root itself).
    pub depth: usize,
    /// Whether this node corresponds to a real file (vs phantom/unresolved).
    pub is_resolved: bool,
}

/// An edge in a neighbourhood subgraph.
#[derive(Debug, Clone, Serialize)]
pub struct SubgraphEdge {
    pub source: String,
    pub target: String,
    pub meta: EdgeMeta,
}

/// The subgraph induced by a BFS neighbourhood around a root node.
#[derive(Debug, Serialize)]
pub struct Subgraph {
    /// The root slug that the neighbourhood was computed from.
    pub root: String,
    /// The BFS depth used.
    pub depth: usize,
    /// All nodes within the neighbourhood, sorted by (depth, slug).
    pub nodes: Vec<SubgraphNode>,
    /// All edges where both endpoints are in the neighbourhood.
    pub edges: Vec<SubgraphEdge>,
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

                let base_meta = EdgeMeta {
                    source_file: file.path.to_string_lossy().to_string(),
                    line: link.line,
                    alias: link.alias.clone(),
                    heading: link.heading.clone(),
                    block_ref: link.block_ref.clone(),
                    is_embed: link.is_embed,
                    predicate: None,
                    annotation: link.annotation.clone(),
                };

                // SPEC-045 REQ-4506: a K-predicate wikilink expands to K
                // directed edges S→X, each carrying one predicate; an untyped
                // link yields a single untyped edge (predicate: None). This
                // keeps an untyped vault byte-identical to the pre-SPEC-045
                // graph (one edge per link occurrence).
                if link.predicates.is_empty() {
                    graph.add_edge(source_idx, target_idx, base_meta);
                } else {
                    for predicate in &link.predicates {
                        let mut meta = base_meta.clone();
                        meta.predicate = Some(predicate.clone());
                        graph.add_edge(source_idx, target_idx, meta);
                    }
                }
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
                    predicate: meta.predicate.clone(),
                    annotation: meta.annotation.clone(),
                }
            })
            .collect()
    }

    /// Whether a real (resolved) page with this name exists, case-insensitively.
    pub fn has_page(&self, page: &str) -> bool {
        self.resolved.iter().any(|p| p.eq_ignore_ascii_case(page))
    }

    /// Enumerate every directed edge in the graph as an [`EdgeRecord`]
    /// (SPEC-045 REQ-4509). Output is sorted deterministically by
    /// `(source, target, predicate, line)` so `zetl edges` is reproducible.
    pub fn all_edges(&self) -> Vec<EdgeRecord> {
        let mut records: Vec<EdgeRecord> = self
            .graph
            .edge_references()
            .map(|edge_ref| {
                let source = self.graph[edge_ref.source()].clone();
                let target_name = &self.graph[edge_ref.target()];
                let meta = edge_ref.weight();
                EdgeRecord {
                    source,
                    target: target_name.clone(),
                    predicate: meta.predicate.clone(),
                    annotation: meta.annotation.clone(),
                    line: meta.line,
                    is_ghost: !self.resolved.contains(target_name),
                }
            })
            .collect();
        records.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then_with(|| a.target.cmp(&b.target))
                .then_with(|| a.predicate.cmp(&b.predicate))
                .then_with(|| a.line.cmp(&b.line))
        });
        records
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
                    predicate: meta.predicate.clone(),
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

    /// Return the set of page names reachable from `anchor` within `depth` hops,
    /// traversing both outgoing and incoming edges (bidirectional BFS).
    ///
    /// The anchor itself is always included in the result.
    /// Returns an error if `anchor` is not found in the graph, with suggestions.
    pub fn neighbourhood(&self, anchor: &str, depth: usize) -> Result<HashSet<String>> {
        // 1. Resolve anchor to a node in the graph
        let &start_idx = self.node_map.get(anchor).ok_or_else(|| {
            let anchor_lower = anchor.to_lowercase();
            let mut similar: Vec<&str> = self
                .node_map
                .keys()
                .filter(|name| {
                    let n = name.to_lowercase();
                    n.contains(&anchor_lower) || anchor_lower.contains(n.as_str())
                })
                .map(|s| s.as_str())
                .collect();
            similar.sort();
            similar.truncate(5);
            if similar.is_empty() {
                anyhow!("Page not found: '{anchor}'")
            } else {
                anyhow!(
                    "Page not found: '{anchor}'. Did you mean: {}",
                    similar.join(", ")
                )
            }
        })?;

        // 2. Initialize visited (by NodeIndex) and queue
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
        visited.insert(start_idx);
        queue.push_back((start_idx, 0));

        // 3. BFS – bidirectional: follow both outgoing and incoming edges
        while let Some((node, d)) = queue.pop_front() {
            // a. If depth reached, skip expanding this node
            if d >= depth {
                continue;
            }

            // b. Explore both edge directions
            let neighbors: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(node, Direction::Outgoing)
                .chain(self.graph.neighbors_directed(node, Direction::Incoming))
                .collect();

            for neighbor in neighbors {
                if visited.insert(neighbor) {
                    queue.push_back((neighbor, d + 1));
                }
            }
        }

        // 4. Convert visited node indices to page names
        Ok(visited
            .into_iter()
            .map(|idx| self.graph[idx].clone())
            .collect())
    }

    /// Return the subgraph induced by a BFS neighbourhood around `root_slug`.
    ///
    /// Performs a bidirectional BFS (both outgoing and incoming edges) up to
    /// `depth` hops from the root. Returns all nodes reached and every edge
    /// where **both** endpoints are inside the neighbourhood.
    ///
    /// This is a pure function — no I/O.
    pub fn filter_neighbourhood(&self, root_slug: &str, depth: usize) -> Result<Subgraph> {
        // 1. Resolve root to a node index
        let &start_idx = self.node_map.get(root_slug).ok_or_else(|| {
            let anchor_lower = root_slug.to_lowercase();
            let mut similar: Vec<&str> = self
                .node_map
                .keys()
                .filter(|name| {
                    let n = name.to_lowercase();
                    n.contains(&anchor_lower) || anchor_lower.contains(n.as_str())
                })
                .map(|s| s.as_str())
                .collect();
            similar.sort();
            similar.truncate(5);
            if similar.is_empty() {
                anyhow!("Page not found: '{root_slug}'")
            } else {
                anyhow!(
                    "Page not found: '{root_slug}'. Did you mean: {}",
                    similar.join(", ")
                )
            }
        })?;

        // 2. BFS — bidirectional, tracking depth per node
        let mut node_depths: HashMap<NodeIndex, usize> = HashMap::new();
        let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
        node_depths.insert(start_idx, 0);
        queue.push_back((start_idx, 0));

        while let Some((node, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            let neighbors: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(node, Direction::Outgoing)
                .chain(self.graph.neighbors_directed(node, Direction::Incoming))
                .collect();

            for neighbor in neighbors {
                if let std::collections::hash_map::Entry::Vacant(e) = node_depths.entry(neighbor) {
                    e.insert(d + 1);
                    queue.push_back((neighbor, d + 1));
                }
            }
        }

        // 3. Build node list
        let neighbourhood_indices: HashSet<NodeIndex> = node_depths.keys().copied().collect();

        let mut nodes: Vec<SubgraphNode> = node_depths
            .iter()
            .map(|(&idx, &d)| {
                let slug = self.graph[idx].clone();
                let is_resolved = self.resolved.contains(&slug);
                SubgraphNode {
                    slug,
                    depth: d,
                    is_resolved,
                }
            })
            .collect();
        nodes.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.slug.cmp(&b.slug)));

        // 4. Collect edges where both endpoints are in the neighbourhood
        let mut edges: Vec<SubgraphEdge> = Vec::new();
        for edge_ref in self.graph.edge_references() {
            let src = edge_ref.source();
            let tgt = edge_ref.target();
            if neighbourhood_indices.contains(&src) && neighbourhood_indices.contains(&tgt) {
                edges.push(SubgraphEdge {
                    source: self.graph[src].clone(),
                    target: self.graph[tgt].clone(),
                    meta: edge_ref.weight().clone(),
                });
            }
        }
        edges.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then_with(|| a.target.cmp(&b.target))
        });

        Ok(Subgraph {
            root: root_slug.to_string(),
            depth,
            nodes,
            edges,
        })
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

// ── Graph index serialisation (CON-101) ────────────────────────────

/// A node in the graph index, pre-computed by the caller.
#[derive(Debug, Clone)]
pub struct GraphIndexNode {
    /// Human-readable page title (e.g. "My Page").
    pub label: String,
    /// URL-safe slug used as the node key (e.g. "my-page").
    pub slug: String,
    pub outlink_count: usize,
    pub backlink_count: usize,
    /// True when the page has zero incoming edges.
    pub is_orphan: bool,
    /// True for phantom/dead-link targets (not backed by a file).
    pub is_dead: bool,
    /// Frontmatter tags; empty vec when none.
    pub tags: Vec<String>,
}

/// A directed edge in the graph index.
#[derive(Debug, Clone)]
pub struct GraphIndexEdge {
    /// Slug of the source node.
    pub source: String,
    /// Slug of the target node.
    pub target: String,
    /// SPEC-045 named-edge predicate (`None` ⇒ untyped edge). When every edge
    /// in the feed is untyped the serialiser stays byte-identical to the
    /// pre-SPEC-045 CON-101 shape.
    pub predicate: Option<String>,
    /// SPEC-045 edge annotation (`None` ⇒ absent).
    pub annotation: Option<String>,
    /// Source line of the link occurrence — used only to disambiguate
    /// graphology edge keys for parallel untyped edges in a typed feed.
    pub line: u32,
}

/// All data needed to produce the graph index JSON.
/// Built by the caller from `VaultData`; the serialiser is pure (no I/O).
pub struct GraphIndexContext<'a> {
    pub vault_name: &'a str,
    pub total_pages: usize,
    pub total_links: usize,
    pub nodes: Vec<GraphIndexNode>,
    pub edges: Vec<GraphIndexEdge>,
}

/// Pure: serialise the vault's link graph to the CON-101 graphology JSON shape
/// (SPEC-028 REQ-101). Stable ordering: nodes and edges sorted alphabetically by
/// their graphology `key` so rebuilds produce deterministic diffs.
///
/// - `page_slug_map` maps real page names to their kebab-case slugs (node keys).
/// - `tags_by_page` carries frontmatter-derived tags per page; pages without an
///   entry receive an empty `tags` array.
/// - `generated_at` is embedded into `attributes.generated_at` verbatim
///   (passed in by the caller so this function remains pure/deterministic).
///
/// Multi-edges in the internal `petgraph::DiGraph` are collapsed per
/// `options.multi = false`; self-loops are preserved (`allowSelfLoops = true`).
pub fn serialize_graph_index(
    graph: &LinkGraph,
    page_slug_map: &HashMap<String, String>,
    tags_by_page: &HashMap<String, Vec<String>>,
    vault_name: &str,
    generated_at: &str,
) -> serde_json::Value {
    let slug_for = |name: &str| -> String {
        page_slug_map
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    };

    let mut node_entries: Vec<(String, serde_json::Value)> =
        Vec::with_capacity(graph.node_map.len());
    for (page_name, &node_idx) in &graph.node_map {
        let is_dead = !graph.resolved.contains(page_name);
        let key = slug_for(page_name);
        let outlink_count = graph
            .graph
            .edges_directed(node_idx, Direction::Outgoing)
            .count();
        let backlink_count = graph
            .graph
            .edges_directed(node_idx, Direction::Incoming)
            .count();
        let is_orphan = !is_dead && backlink_count == 0;
        let tags = tags_by_page.get(page_name).cloned().unwrap_or_default();
        let attrs = serde_json::json!({
            "label": page_name,
            "slug": key,
            "outlink_count": outlink_count,
            "backlink_count": backlink_count,
            "is_orphan": is_orphan,
            "is_dead": is_dead,
            "tags": tags,
        });
        node_entries.push((
            key.clone(),
            serde_json::json!({ "key": key, "attributes": attrs }),
        ));
    }
    node_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let nodes_out: Vec<serde_json::Value> = node_entries.into_iter().map(|(_, v)| v).collect();

    let mut seen_edges: HashSet<(String, String)> = HashSet::new();
    let mut edge_entries: Vec<(String, serde_json::Value)> = Vec::new();
    for edge_ref in graph.graph.edge_references() {
        let source_name = &graph.graph[edge_ref.source()];
        let target_name = &graph.graph[edge_ref.target()];
        let source_slug = slug_for(source_name);
        let target_slug = slug_for(target_name);
        let pair = (source_slug.clone(), target_slug.clone());
        if !seen_edges.insert(pair) {
            continue;
        }
        let key = format!("{source_slug}->{target_slug}");
        edge_entries.push((
            key.clone(),
            serde_json::json!({
                "key": key,
                "source": source_slug,
                "target": target_slug,
                "attributes": {},
            }),
        ));
    }
    edge_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let edges_out: Vec<serde_json::Value> = edge_entries.into_iter().map(|(_, v)| v).collect();

    let pages_count = graph.resolved.len();
    let links_count = edges_out.len();

    serde_json::json!({
        "options": {
            "type": "directed",
            "multi": false,
            "allowSelfLoops": true,
        },
        "attributes": {
            "format": "zetl-graph/v1",
            "generated_at": generated_at,
            "vault": {
                "name": vault_name,
                "pages": pages_count,
                "links": links_count,
            }
        },
        "nodes": nodes_out,
        "edges": edges_out,
    })
}

/// Context-based variant of [`serialize_graph_index`] that takes a pre-built
/// [`GraphIndexContext`] instead of raw arguments. Used by `zetl serve` route
/// handlers via `build_graph_index_context`.
pub fn serialize_graph_index_ctx(ctx: &GraphIndexContext) -> serde_json::Value {
    let mut nodes = ctx.nodes.clone();
    nodes.sort_by(|a, b| a.slug.cmp(&b.slug));

    let nodes_json: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            serde_json::json!({
                "key": n.slug,
                "attributes": {
                    "label": n.label,
                    "slug": n.slug,
                    "outlink_count": n.outlink_count,
                    "backlink_count": n.backlink_count,
                    "is_orphan": n.is_orphan,
                    "is_dead": n.is_dead,
                    "tags": n.tags,
                }
            })
        })
        .collect();

    // SPEC-045 REQ-4517 / CON-4508: the feed is a backward-compatible SUPERSET
    // of the CON-101 graphology shape. When the vault has NO typed edges the
    // output is BYTE-IDENTICAL to the pre-SPEC-045 feed — the untyped branch
    // below reproduces it verbatim (no `multi`, no predicate-qualified keys, no
    // `predicate`/`annotation` attributes). When the vault HAS typed edges we
    // switch to a multigraph: parallel edges between the same (source,target)
    // become distinct entries keyed by `(source, target, predicate)`, each
    // carrying `predicate`/`annotation` under `attributes`, and `options.multi`
    // is flipped to `true` so graphology accepts the parallel edges.
    let has_typed = ctx.edges.iter().any(|e| e.predicate.is_some());

    if !has_typed {
        // ── Untyped: pre-SPEC-045 CON-101 path, kept verbatim. ──
        let mut edges = ctx.edges.clone();
        edges.sort_by(|a, b| {
            let key_a = format!("{}->{}", a.source, a.target);
            let key_b = format!("{}->{}", b.source, b.target);
            key_a.cmp(&key_b)
        });
        edges.dedup_by(|a, b| a.source == b.source && a.target == b.target);

        let edges_json: Vec<serde_json::Value> = edges
            .iter()
            .map(|e| {
                serde_json::json!({
                    "key": format!("{}->{}", e.source, e.target),
                    "source": e.source,
                    "target": e.target,
                    "attributes": {}
                })
            })
            .collect();

        return serde_json::json!({
            "attributes": {
                "format": "zetl-graph/v1",
                "vault": {
                    "name": ctx.vault_name,
                    "pages": ctx.total_pages,
                    "links": ctx.total_links,
                }
            },
            "options": {
                "type": "directed",
                "multi": false,
                "allowSelfLoops": true,
            },
            "nodes": nodes_json,
            "edges": edges_json,
        });
    }

    // ── Typed: multigraph superset. ──
    // Identity is (source, target, predicate). A K-predicate wikilink yields K
    // parallel edges; untyped edges between the same pair are de-duplicated to
    // a single entry (predicate = None). The key is predicate-qualified, and a
    // monotonic suffix guarantees uniqueness should the same identity recur.
    let mut edges = ctx.edges.clone();
    edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.predicate.cmp(&b.predicate))
            .then_with(|| a.line.cmp(&b.line))
    });
    // Collapse exact (source, target, predicate) duplicates — a typed edge is
    // identified by that triple, not by line.
    edges.dedup_by(|a, b| {
        a.source == b.source && a.target == b.target && a.predicate == b.predicate
    });

    let mut used_keys: HashSet<String> = HashSet::new();
    let edges_json: Vec<serde_json::Value> = edges
        .iter()
        .map(|e| {
            let pred_part = e.predicate.as_deref().unwrap_or("__untyped");
            let mut key = format!("{}--{}--{}", e.source, e.target, pred_part);
            // Defensive uniqueness guard (should not trigger after the dedup
            // above, but keeps keys unique under any input).
            if used_keys.contains(&key) {
                let mut suffix = 1u32;
                loop {
                    let candidate = format!("{key}#{suffix}");
                    if !used_keys.contains(&candidate) {
                        key = candidate;
                        break;
                    }
                    suffix += 1;
                }
            }
            used_keys.insert(key.clone());
            let predicate = e
                .predicate
                .as_ref()
                .map(|p| serde_json::Value::String(p.clone()))
                .unwrap_or(serde_json::Value::Null);
            let annotation = e
                .annotation
                .as_ref()
                .map(|a| serde_json::Value::String(a.clone()))
                .unwrap_or(serde_json::Value::Null);
            serde_json::json!({
                "key": key,
                "source": e.source,
                "target": e.target,
                "attributes": {
                    "predicate": predicate,
                    "annotation": annotation,
                }
            })
        })
        .collect();

    serde_json::json!({
        "attributes": {
            "format": "zetl-graph/v1",
            "vault": {
                "name": ctx.vault_name,
                "pages": ctx.total_pages,
                "links": ctx.total_links,
            }
        },
        "options": {
            "type": "directed",
            "multi": true,
            "allowSelfLoops": true,
        },
        "nodes": nodes_json,
        "edges": edges_json,
    })
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
                    predicates: Vec::new(),
                    annotation: None,
                    line,
                    column: 1,
                })
                .collect(),
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    /// Helper: build a ParsedFile whose links carry SPEC-045 predicates.
    /// Each tuple is `(target, line, predicates)`.
    fn make_typed_file(name: &str, links: Vec<(&str, u32, Vec<&str>)>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(format!("{name}.md")),
            page_name: name.to_string(),
            links: links
                .into_iter()
                .map(|(target, line, preds)| crate::types::WikiLink {
                    target_page: target.to_string(),
                    raw_target: target.to_string(),
                    heading: None,
                    block_ref: None,
                    alias: None,
                    is_embed: false,
                    predicates: preds.into_iter().map(|p| p.to_string()).collect(),
                    annotation: None,
                    line,
                    column: 1,
                })
                .collect(),
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    #[test]
    fn spec045_k_predicate_link_expands_to_k_edges() {
        // `derived_from::informed_by::[[Retro]]` on "Decision Log" → two
        // directed typed edges, plus one untyped edge for a bare link.
        let mut resolved = HashMap::new();
        resolved.insert("retro".to_string(), "Retro".to_string());
        resolved.insert("loose".to_string(), "Loose".to_string());
        let files = vec![make_typed_file(
            "Decision Log",
            vec![
                ("Retro", 3, vec!["derived_from", "informed_by"]),
                ("Loose", 5, vec![]), // untyped
            ],
        )];
        let graph = LinkGraph::build(&files, &resolved);
        let fwd = graph.forward_links("Decision Log");
        assert_eq!(fwd.len(), 3, "2 typed + 1 untyped");

        let mut typed: Vec<_> = fwd
            .iter()
            .filter_map(|f| f.meta.predicate.as_deref().map(|p| (p, f.target.as_str())))
            .collect();
        typed.sort();
        assert_eq!(
            typed,
            vec![("derived_from", "Retro"), ("informed_by", "Retro")]
        );
        // The bare link stays untyped.
        assert_eq!(
            fwd.iter().filter(|f| f.meta.predicate.is_none()).count(),
            1
        );
    }

    #[test]
    fn spec045_typed_backlink_carries_predicate() {
        let mut resolved = HashMap::new();
        resolved.insert("move fast doctrine".to_string(), "Move Fast Doctrine".to_string());
        let files = vec![
            make_typed_file(
                "Decision Log",
                vec![("Move Fast Doctrine", 7, vec!["contradicts"])],
            ),
            make_typed_file("Move Fast Doctrine", vec![]),
        ];
        let graph = LinkGraph::build(&files, &resolved);
        let bl = graph.backlinks("Move Fast Doctrine");
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].source, "Decision Log");
        assert_eq!(bl[0].predicate.as_deref(), Some("contradicts"));
    }

    #[test]
    fn spec045_dead_typed_edge_reports_predicate() {
        // A typed edge to a phantom target is a typed ghost edge (REQ-4514).
        let resolved = HashMap::new();
        let files = vec![make_typed_file(
            "Plan",
            vec![("Nonexistent", 2, vec!["supersedes"])],
        )];
        let graph = LinkGraph::build(&files, &resolved);
        let dead = graph.dead_links();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].target, "Nonexistent");
        assert_eq!(dead[0].predicate.as_deref(), Some("supersedes"));
    }

    // ── SPEC-045 REQ-4517 / CON-4508: graph-feed superset ───────────────────

    /// Helper: an untyped edge for the graph-index context.
    fn idx_edge(source: &str, target: &str) -> GraphIndexEdge {
        GraphIndexEdge {
            source: source.to_string(),
            target: target.to_string(),
            predicate: None,
            annotation: None,
            line: 1,
        }
    }

    fn empty_ctx<'a>(edges: Vec<GraphIndexEdge>) -> GraphIndexContext<'a> {
        GraphIndexContext {
            vault_name: "Vault",
            total_pages: 2,
            total_links: edges.len(),
            nodes: vec![
                GraphIndexNode {
                    label: "A".to_string(),
                    slug: "a".to_string(),
                    outlink_count: 1,
                    backlink_count: 0,
                    is_orphan: true,
                    is_dead: false,
                    tags: vec![],
                },
                GraphIndexNode {
                    label: "B".to_string(),
                    slug: "b".to_string(),
                    outlink_count: 0,
                    backlink_count: 1,
                    is_orphan: false,
                    is_dead: false,
                    tags: vec![],
                },
            ],
            edges,
        }
    }

    #[test]
    fn spec045_feed_no_typed_edges_is_con101_byte_identical() {
        // An untyped feed must reproduce the pre-SPEC-045 CON-101 output exactly:
        // multi == false, edge keys are `<src>-><tgt>`, and edge attributes are
        // the empty object (no `predicate`/`annotation`).
        let ctx = empty_ctx(vec![idx_edge("a", "b")]);
        let val = serialize_graph_index_ctx(&ctx);

        assert_eq!(val["options"]["multi"], serde_json::json!(false));
        let edges = val["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["key"], "a->b");
        assert_eq!(edges[0]["source"], "a");
        assert_eq!(edges[0]["target"], "b");
        // Attributes must be the empty object — no predicate key present.
        assert_eq!(edges[0]["attributes"], serde_json::json!({}));
        assert!(edges[0]["attributes"].get("predicate").is_none());
    }

    #[test]
    fn spec045_feed_typed_edge_flips_multi_and_carries_predicate() {
        let ctx = empty_ctx(vec![GraphIndexEdge {
            source: "a".to_string(),
            target: "b".to_string(),
            predicate: Some("contradicts".to_string()),
            annotation: Some("see §3".to_string()),
            line: 7,
        }]);
        let val = serialize_graph_index_ctx(&ctx);

        assert_eq!(val["options"]["multi"], serde_json::json!(true));
        let edges = val["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["attributes"]["predicate"], "contradicts");
        assert_eq!(edges[0]["attributes"]["annotation"], "see §3");
        assert_eq!(edges[0]["key"], "a--b--contradicts");
    }

    #[test]
    fn spec045_feed_k_predicate_produces_k_parallel_edges_distinct_keys() {
        // Two predicates on the same (a,b) pair → two parallel edges, distinct
        // keys, both present in a multigraph feed.
        let ctx = empty_ctx(vec![
            GraphIndexEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                predicate: Some("derived_from".to_string()),
                annotation: None,
                line: 3,
            },
            GraphIndexEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                predicate: Some("informed_by".to_string()),
                annotation: None,
                line: 3,
            },
        ]);
        let val = serialize_graph_index_ctx(&ctx);

        assert_eq!(val["options"]["multi"], serde_json::json!(true));
        let edges = val["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2);
        let mut keys: Vec<&str> = edges.iter().map(|e| e["key"].as_str().unwrap()).collect();
        keys.sort();
        assert_eq!(keys, vec!["a--b--derived_from", "a--b--informed_by"]);
        // All endpoints unchanged for plain source/target consumers.
        for e in edges {
            assert_eq!(e["source"], "a");
            assert_eq!(e["target"], "b");
        }
    }

    #[test]
    fn spec045_feed_mixed_typed_and_untyped_keys_unique() {
        // A typed feed that also contains an untyped edge between the same pair:
        // the untyped edge gets the `__untyped` key segment and a null predicate.
        let ctx = empty_ctx(vec![
            GraphIndexEdge {
                source: "a".to_string(),
                target: "b".to_string(),
                predicate: Some("contradicts".to_string()),
                annotation: None,
                line: 7,
            },
            idx_edge("a", "b"), // untyped, same pair
        ]);
        let val = serialize_graph_index_ctx(&ctx);

        let edges = val["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 2);
        let keys: HashSet<&str> = edges.iter().map(|e| e["key"].as_str().unwrap()).collect();
        assert_eq!(keys.len(), 2, "keys must be unique");
        assert!(keys.contains("a--b--contradicts"));
        assert!(keys.contains("a--b--__untyped"));
        // The untyped parallel edge carries a null predicate (not the empty obj).
        let untyped = edges
            .iter()
            .find(|e| e["key"] == "a--b--__untyped")
            .unwrap();
        assert_eq!(untyped["attributes"]["predicate"], serde_json::Value::Null);
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
            predicates: Vec::new(),
            annotation: None,
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
            merkle_leaves: vec![],
            file_merkle: None,
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
                merkle_leaves: vec![],
                file_merkle: None,
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
            merkle_leaves: vec![],
            file_merkle: None,
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
            merkle_leaves: vec![],
            file_merkle: None,
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

    // ── neighbourhood() tests ────────────────────────────────────────────────

    #[test]
    fn test_neighbourhood_depth0_returns_only_anchor() {
        let graph = simple_graph(); // A->B, A->C, B->C, C->A
        let result = graph.neighbourhood("A", 0).unwrap();
        assert_eq!(result, HashSet::from(["A".to_string()]));
    }

    #[test]
    fn test_neighbourhood_depth1_outgoing() {
        // Chain: A -> B -> C (no back-edges)
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

        // From A at depth 1: A + outgoing neighbour B + incoming (none) = {A, B}
        let result = graph.neighbourhood("A", 1).unwrap();
        assert_eq!(result, HashSet::from(["A".to_string(), "B".to_string()]));
    }

    #[test]
    fn test_neighbourhood_depth1_bidirectional() {
        // A -> B, C -> B  (B has two incoming edges)
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![]),
            make_file("C", vec![("B", 1)]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        // From B at depth 1: B + outgoing (none) + incoming {A, C} = {A, B, C}
        let result = graph.neighbourhood("B", 1).unwrap();
        assert_eq!(
            result,
            HashSet::from(["A".to_string(), "B".to_string(), "C".to_string()])
        );
    }

    #[test]
    fn test_neighbourhood_depth2_chain() {
        // A -> B -> C -> D
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

        // From A at depth 2: A -> B (d=1) -> C (d=2), C is visited but not expanded
        let result = graph.neighbourhood("A", 2).unwrap();
        assert_eq!(
            result,
            HashSet::from(["A".to_string(), "B".to_string(), "C".to_string()])
        );

        // From B at depth 2 (bidirectional): B <- A (d=1) -> nothing new;
        //   B -> C (d=1) -> D (d=2)
        let result2 = graph.neighbourhood("B", 2).unwrap();
        assert_eq!(
            result2,
            HashSet::from([
                "A".to_string(),
                "B".to_string(),
                "C".to_string(),
                "D".to_string()
            ])
        );
    }

    #[test]
    fn test_neighbourhood_anchor_always_included() {
        let graph = simple_graph();
        for depth in [0, 1, 2, 5] {
            let result = graph.neighbourhood("B", depth).unwrap();
            assert!(result.contains("B"), "anchor missing at depth {depth}");
        }
    }

    #[test]
    fn test_neighbourhood_not_found_error() {
        let graph = simple_graph();
        let err = graph.neighbourhood("NoSuchPage", 2).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not found"), "expected 'not found' in: {msg}");
        assert!(msg.contains("NoSuchPage"), "expected page name in: {msg}");
    }

    #[test]
    fn test_neighbourhood_not_found_suggests_similar() {
        // Graph with page "Alpha" — querying "alpha" (lowercase) should suggest it
        let files = vec![make_file("Alpha", vec![])];
        let resolved: HashMap<String, String> = [("Alpha".to_string(), "Alpha".to_string())]
            .into_iter()
            .collect();
        let graph = LinkGraph::build(&files, &resolved);

        let err = graph.neighbourhood("alpha", 1).unwrap_err();
        let msg = err.to_string();
        // "Alpha" should appear in the suggestion list
        assert!(msg.contains("Alpha"), "expected suggestion in: {msg}");
    }

    #[test]
    fn test_neighbourhood_cycle_terminates() {
        // A -> B -> C -> A (cycle)
        let graph = simple_graph();
        // Should terminate without infinite loop
        let result = graph.neighbourhood("A", 10).unwrap();
        // All three nodes reachable
        assert_eq!(
            result,
            HashSet::from(["A".to_string(), "B".to_string(), "C".to_string()])
        );
    }

    #[test]
    fn test_neighbourhood_isolated_node() {
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![]),
            make_file("Isolated", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("Isolated".to_string(), "Isolated".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        let result = graph.neighbourhood("Isolated", 5).unwrap();
        assert_eq!(result, HashSet::from(["Isolated".to_string()]));
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
            merkle_leaves: vec![],
            file_merkle: None,
        }];
        let target_file = ParsedFile {
            path: PathBuf::from("target.md"),
            page_name: "target".to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
            merkle_leaves: vec![],
            file_merkle: None,
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

    // ── filter_neighbourhood() tests ────────────────────────────────────────

    #[test]
    fn test_filter_neighbourhood_depth0_root_only() {
        let graph = simple_graph();
        let sub = graph.filter_neighbourhood("A", 0).unwrap();
        assert_eq!(sub.root, "A");
        assert_eq!(sub.depth, 0);
        assert_eq!(sub.nodes.len(), 1);
        assert_eq!(sub.nodes[0].slug, "A");
        assert_eq!(sub.nodes[0].depth, 0);
        assert!(sub.nodes[0].is_resolved);
        // A has self-loop? No — simple_graph has A->B, A->C, B->C, C->A.
        // Only edge with both endpoints in {A} would be a self-loop, which doesn't exist.
        assert!(sub.edges.is_empty());
    }

    #[test]
    fn test_filter_neighbourhood_depth1_includes_edges() {
        // A -> B -> C (chain, no back-edges)
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

        let sub = graph.filter_neighbourhood("A", 1).unwrap();
        let slugs: Vec<&str> = sub.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert_eq!(slugs, vec!["A", "B"]); // sorted by (depth, slug)

        assert_eq!(sub.nodes[0].depth, 0); // A is root
        assert_eq!(sub.nodes[1].depth, 1); // B is depth 1

        // Edge A->B should be present; B->C should NOT (C not in neighbourhood)
        assert_eq!(sub.edges.len(), 1);
        assert_eq!(sub.edges[0].source, "A");
        assert_eq!(sub.edges[0].target, "B");
    }

    #[test]
    fn test_filter_neighbourhood_bidirectional_edges() {
        // A -> B, C -> B
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![]),
            make_file("C", vec![("B", 1)]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("C".to_string(), "C".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        // From B depth 1: reaches A (incoming) and C (incoming)
        let sub = graph.filter_neighbourhood("B", 1).unwrap();
        let slugs: Vec<&str> = sub.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert_eq!(slugs, vec!["B", "A", "C"]); // B@0, then A@1, C@1

        // Both A->B and C->B edges should be included
        assert_eq!(sub.edges.len(), 2);
    }

    #[test]
    fn test_filter_neighbourhood_full_cycle() {
        let graph = simple_graph(); // A->B, A->C, B->C, C->A
        let sub = graph.filter_neighbourhood("A", 10).unwrap();

        // All 3 nodes reachable
        assert_eq!(sub.nodes.len(), 3);
        // All 4 edges should be present (both endpoints always in the set)
        assert_eq!(sub.edges.len(), 4);
    }

    #[test]
    fn test_filter_neighbourhood_phantom_node() {
        let files = vec![make_file("A", vec![("Ghost", 1)])];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let sub = graph.filter_neighbourhood("A", 1).unwrap();
        assert_eq!(sub.nodes.len(), 2);

        let ghost_node = sub.nodes.iter().find(|n| n.slug == "Ghost").unwrap();
        assert!(!ghost_node.is_resolved);
        assert_eq!(ghost_node.depth, 1);

        let a_node = sub.nodes.iter().find(|n| n.slug == "A").unwrap();
        assert!(a_node.is_resolved);
    }

    #[test]
    fn test_filter_neighbourhood_not_found() {
        let graph = simple_graph();
        let err = graph.filter_neighbourhood("NoSuchPage", 1).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_filter_neighbourhood_isolated_node() {
        let files = vec![
            make_file("A", vec![("B", 1)]),
            make_file("B", vec![]),
            make_file("Isolated", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
            ("Isolated".to_string(), "Isolated".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);

        let sub = graph.filter_neighbourhood("Isolated", 5).unwrap();
        assert_eq!(sub.nodes.len(), 1);
        assert_eq!(sub.nodes[0].slug, "Isolated");
        assert!(sub.edges.is_empty());
    }

    #[test]
    fn test_filter_neighbourhood_nodes_sorted_by_depth_then_slug() {
        // A -> B -> C -> D
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

        let sub = graph.filter_neighbourhood("B", 2).unwrap();
        // B@0, A@1 (incoming), C@1 (outgoing), D@2
        let pairs: Vec<(&str, usize)> = sub
            .nodes
            .iter()
            .map(|n| (n.slug.as_str(), n.depth))
            .collect();
        assert_eq!(pairs, vec![("B", 0), ("A", 1), ("C", 1), ("D", 2)]);
    }

    #[test]
    fn test_filter_neighbourhood_edges_sorted() {
        let graph = simple_graph(); // A->B, A->C, B->C, C->A
        let sub = graph.filter_neighbourhood("A", 10).unwrap();

        let edge_pairs: Vec<(&str, &str)> = sub
            .edges
            .iter()
            .map(|e| (e.source.as_str(), e.target.as_str()))
            .collect();
        // Sorted by (source, target)
        assert_eq!(
            edge_pairs,
            vec![("A", "B"), ("A", "C"), ("B", "C"), ("C", "A")]
        );
    }

    #[test]
    fn test_filter_neighbourhood_edge_meta_preserved() {
        let files = [
            ParsedFile {
                path: PathBuf::from("source.md"),
                page_name: "source".to_string(),
                links: vec![make_link(
                    "target",
                    7,
                    Some("alias"),
                    Some("heading"),
                    None,
                    true,
                )],
                spl_blocks: vec![],
                diagnostics: vec![],
                mtime: SystemTime::now(),
                merkle_leaves: vec![],
                file_merkle: None,
            },
            ParsedFile {
                path: PathBuf::from("target.md"),
                page_name: "target".to_string(),
                links: vec![],
                spl_blocks: vec![],
                diagnostics: vec![],
                mtime: SystemTime::now(),
                merkle_leaves: vec![],
                file_merkle: None,
            },
        ];
        let resolved: HashMap<String, String> = [("target".to_string(), "target".to_string())]
            .into_iter()
            .collect();
        let graph = LinkGraph::build(&files, &resolved);

        let sub = graph.filter_neighbourhood("source", 1).unwrap();
        assert_eq!(sub.edges.len(), 1);
        let e = &sub.edges[0];
        assert_eq!(e.meta.line, 7);
        assert_eq!(e.meta.alias.as_deref(), Some("alias"));
        assert_eq!(e.meta.heading.as_deref(), Some("heading"));
        assert!(e.meta.is_embed);
    }

    // ── SPEC-028 REQ-101 / CON-101: serialize_graph_index ─────────────

    #[test]
    fn serialize_graph_index_shape_and_ordering() {
        let graph = simple_graph();
        let slug_map: HashMap<String, String> = [
            ("A".to_string(), "a".to_string()),
            ("B".to_string(), "b".to_string()),
            ("C".to_string(), "c".to_string()),
        ]
        .into_iter()
        .collect();
        let tags: HashMap<String, Vec<String>> = [("A".to_string(), vec!["rust".to_string()])]
            .into_iter()
            .collect();

        let out =
            serialize_graph_index(&graph, &slug_map, &tags, "my-vault", "2026-04-16T00:00:00Z");

        assert_eq!(out["options"]["type"], "directed");
        assert_eq!(out["options"]["multi"], false);
        assert_eq!(out["options"]["allowSelfLoops"], true);
        assert_eq!(out["attributes"]["format"], "zetl-graph/v1");
        assert_eq!(out["attributes"]["generated_at"], "2026-04-16T00:00:00Z");
        assert_eq!(out["attributes"]["vault"]["name"], "my-vault");
        assert_eq!(out["attributes"]["vault"]["pages"], 3);
        assert_eq!(out["attributes"]["vault"]["links"], 4);

        let nodes = out["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3);
        // Stable alphabetical ordering by slug (= key).
        let keys: Vec<&str> = nodes.iter().map(|n| n["key"].as_str().unwrap()).collect();
        assert_eq!(keys, vec!["a", "b", "c"]);
        // Node A carries its frontmatter tag; B and C get empty arrays.
        assert_eq!(nodes[0]["attributes"]["tags"][0], "rust");
        assert_eq!(nodes[1]["attributes"]["tags"].as_array().unwrap().len(), 0);
        assert_eq!(nodes[0]["attributes"]["label"], "A");
        assert_eq!(nodes[0]["attributes"]["is_dead"], false);
        // A has a backlink from C → not an orphan.
        assert_eq!(nodes[0]["attributes"]["is_orphan"], false);

        let edges = out["edges"].as_array().unwrap();
        // A→B, A→C, B→C, C→A sorted alphabetically by "source->target".
        let edge_keys: Vec<&str> = edges.iter().map(|e| e["key"].as_str().unwrap()).collect();
        assert_eq!(edge_keys, vec!["a->b", "a->c", "b->c", "c->a"]);
    }

    #[test]
    fn serialize_graph_index_marks_dead_and_orphan_nodes() {
        // A → B (real) and A → Ghost (phantom/dead). A has no backlinks → orphan.
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
        let slug_map: HashMap<String, String> = [
            ("A".to_string(), "a".to_string()),
            ("B".to_string(), "b".to_string()),
        ]
        .into_iter()
        .collect();

        let out = serialize_graph_index(
            &graph,
            &slug_map,
            &HashMap::new(),
            "v",
            "2026-04-16T00:00:00Z",
        );

        let nodes = out["nodes"].as_array().unwrap();
        let by_key: HashMap<&str, &serde_json::Value> = nodes
            .iter()
            .map(|n| (n["key"].as_str().unwrap(), n))
            .collect();

        // A is a real page with no backlinks → is_orphan=true, is_dead=false.
        assert_eq!(by_key["a"]["attributes"]["is_orphan"], true);
        assert_eq!(by_key["a"]["attributes"]["is_dead"], false);
        // Ghost is a phantom — key defaults to page name since there's no slug entry.
        assert_eq!(by_key["Ghost"]["attributes"]["is_dead"], true);
        assert_eq!(by_key["Ghost"]["attributes"]["is_orphan"], false);
        // Links count should reflect all outgoing edges (dedup — 2 distinct edges).
        assert_eq!(out["attributes"]["vault"]["links"], 2);
        // Pages count counts only real pages.
        assert_eq!(out["attributes"]["vault"]["pages"], 2);
    }

    #[test]
    fn serialize_graph_index_dedupes_multi_edges() {
        // A → B appears twice in the file; graphology multi=false expects one edge.
        let files = vec![
            make_file("A", vec![("B", 1), ("B", 2)]),
            make_file("B", vec![]),
        ];
        let resolved: HashMap<String, String> = [
            ("A".to_string(), "A".to_string()),
            ("B".to_string(), "B".to_string()),
        ]
        .into_iter()
        .collect();
        let graph = LinkGraph::build(&files, &resolved);
        let slug_map: HashMap<String, String> = [
            ("A".to_string(), "a".to_string()),
            ("B".to_string(), "b".to_string()),
        ]
        .into_iter()
        .collect();

        let out = serialize_graph_index(
            &graph,
            &slug_map,
            &HashMap::new(),
            "v",
            "2026-04-16T00:00:00Z",
        );

        let edges = out["edges"].as_array().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0]["key"], "a->b");
        assert_eq!(out["attributes"]["vault"]["links"], 1);
    }
}
