pub mod markdown;
mod ui;

use std::collections::{HashMap, HashSet};
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use regex::Regex;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::graph::{DeadLink, LinkGraph, Orphan};
use crate::types::{Diagnostic, ParsedFile};

// ── Timeline navigation types ─────────────────────────────────────────────

/// Metadata for a single historical snapshot shown in the timeline.
#[derive(Debug, Clone)]
pub struct SnapshotLabel {
    /// Short change-ID string (12 chars).
    pub change_id: String,
    /// Human-readable timestamp, e.g. "2026-03-04 14:30".
    pub timestamp_display: String,
    /// Unix timestamp in seconds (for day-boundary arithmetic).
    pub timestamp_unix: i64,
    /// Whether a cached index exists for this snapshot.
    pub has_cached_index: bool,
}

/// Loader function: given snapshot index → files + graph for that snapshot.
pub type HistoricalLoader = Box<dyn Fn(usize) -> anyhow::Result<(Vec<ParsedFile>, LinkGraph)>>;

// ── Tab enum ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Pages,
    Links,
    Search,
    Diagnostics,
    PageDetail,
    Graph,
}

impl Tab {
    const ALL: [Tab; 7] = [
        Tab::Dashboard,
        Tab::Pages,
        Tab::Links,
        Tab::Search,
        Tab::Diagnostics,
        Tab::PageDetail,
        Tab::Graph,
    ];

    fn label(&self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Pages => "Pages",
            Tab::Links => "Links",
            Tab::Search => "Search",
            Tab::Diagnostics => "Diagnostics",
            Tab::PageDetail => "Page",
            Tab::Graph => "Graph",
        }
    }

    fn index(&self) -> usize {
        match self {
            Tab::Dashboard => 0,
            Tab::Pages => 1,
            Tab::Links => 2,
            Tab::Search => 3,
            Tab::Diagnostics => 4,
            Tab::PageDetail => 5,
            Tab::Graph => 6,
        }
    }
}

// ── App state ─────────────────────────────────────────────────────────────

pub struct App {
    pub active_tab: Tab,
    pub should_quit: bool,
    pub show_help: bool,

    // Data
    pub files: Vec<ParsedFile>,
    pub graph: LinkGraph,
    pub vault_root: PathBuf,
    pub page_names: Vec<String>,

    // Precomputed diagnostics
    pub dead_links: Vec<DeadLink>,
    pub orphans: Vec<Orphan>,
    pub diagnostics: Vec<Diagnostic>,

    // Pages tab
    pub page_filter: Input,
    pub page_selected: usize,
    pub page_scroll_offset: usize,

    // Links tab (with context)
    pub link_page_input: Input,
    pub link_selected_page: Option<String>,
    pub link_fwd: Vec<LinkEntry>,
    pub link_back: Vec<LinkEntry>,
    pub link_pane: LinkPane,
    pub link_fwd_selected: usize,
    pub link_back_selected: usize,

    // Search tab
    pub search_input: Input,
    pub search_results: Vec<SearchResult>,
    pub search_selected: usize,
    pub search_active: bool,

    // Diagnostics tab
    pub diag_selected: usize,
    pub diag_category: DiagCategory,

    // Page Detail tab
    pub detail_page: Option<String>,
    pub detail_content: Vec<Line<'static>>,
    pub detail_frontmatter: Vec<(String, String)>,
    pub detail_backlinks: Vec<LinkEntry>,
    pub detail_unlinked: Vec<UnlinkedMention>,
    pub detail_scroll: usize,
    pub detail_pane: DetailPane,
    pub detail_sidebar_scroll: usize,
    pub detail_wikilinks: Vec<ContentWikilink>, // wikilinks found in rendered content
    pub detail_wikilink_idx: Option<usize>,     // selected wikilink index

    // Local Graph tab
    pub graph_page: Option<String>,
    pub graph_depth: usize, // 1 or 2
    pub graph_outgoing: Vec<GraphNeighbor>,
    pub graph_incoming: Vec<GraphNeighbor>,
    pub graph_pane: LinkPane,
    pub graph_out_selected: usize,
    pub graph_in_selected: usize,

    // Quick switcher (Ctrl+K overlay)
    pub switcher_open: bool,
    pub switcher_input: Input,
    pub switcher_selected: usize,

    // Breadcrumb navigation history
    pub history: Vec<String>,

    // Timeline navigation (REQ-091)
    /// All known snapshots (newest-first). Empty when history is unavailable.
    pub timeline_snapshots: Vec<SnapshotLabel>,
    /// Index into `timeline_snapshots` for the currently viewed snapshot.
    /// `None` = live view.
    pub timeline_idx: Option<usize>,
    /// Loader function that produces files + graph for a given snapshot index.
    historical_loader: Option<HistoricalLoader>,
    /// Saved live data (files + graph) for restoring when leaving historical mode.
    live_snapshot: Option<(Vec<ParsedFile>, LinkGraph)>,
}

#[derive(Debug, Clone)]
pub struct LinkEntry {
    pub page: String,
    pub line: u32,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPane {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailPane {
    Content,
    Sidebar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagCategory {
    DeadLinks,
    Orphans,
    Syntax,
}

impl DiagCategory {
    fn next(&self) -> Self {
        match self {
            DiagCategory::DeadLinks => DiagCategory::Orphans,
            DiagCategory::Orphans => DiagCategory::Syntax,
            DiagCategory::Syntax => DiagCategory::DeadLinks,
        }
    }

    fn prev(&self) -> Self {
        match self {
            DiagCategory::DeadLinks => DiagCategory::Syntax,
            DiagCategory::Orphans => DiagCategory::DeadLinks,
            DiagCategory::Syntax => DiagCategory::Orphans,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub page: String,
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnlinkedMention {
    pub page: String,
    pub line: u32,
    pub context: String,
}

/// A wikilink found in the rendered markdown content, with its position in rendered lines.
#[derive(Debug, Clone)]
pub struct ContentWikilink {
    pub rendered_line: usize, // line index in detail_content
    pub target: String,       // target page name
    pub match_on_line: usize, // 0-based index of this wikilink among all wikilinks on this line
}

#[derive(Debug, Clone)]
pub struct GraphNeighbor {
    pub page: String,
    pub link_count: usize,
    pub children: Vec<(String, usize)>, // 2nd-hop neighbors
}

impl App {
    pub fn new(
        files: Vec<ParsedFile>,
        file_index: Vec<(String, PathBuf)>,
        graph: LinkGraph,
        vault_root: PathBuf,
        resolved_pages: &HashMap<String, String>,
    ) -> Self {
        let _ = (file_index, resolved_pages);

        let mut page_names: Vec<String> = files.iter().map(|f| f.page_name.clone()).collect();
        page_names.sort_by_key(|a| a.to_lowercase());

        let dead_links = graph.dead_links();
        let orphans = graph.orphans();
        let diagnostics: Vec<Diagnostic> =
            files.iter().flat_map(|f| f.diagnostics.clone()).collect();

        App {
            active_tab: Tab::Dashboard,
            should_quit: false,
            show_help: false,

            files,
            graph,
            vault_root,
            page_names,

            dead_links,
            orphans,
            diagnostics,

            page_filter: Input::default(),
            page_selected: 0,
            page_scroll_offset: 0,

            link_page_input: Input::default(),
            link_selected_page: None,
            link_fwd: Vec::new(),
            link_back: Vec::new(),
            link_pane: LinkPane::Forward,
            link_fwd_selected: 0,
            link_back_selected: 0,

            search_input: Input::default(),
            search_results: Vec::new(),
            search_selected: 0,
            search_active: true,

            diag_selected: 0,
            diag_category: DiagCategory::DeadLinks,

            detail_page: None,
            detail_content: Vec::new(),
            detail_frontmatter: Vec::new(),
            detail_backlinks: Vec::new(),
            detail_unlinked: Vec::new(),
            detail_scroll: 0,
            detail_pane: DetailPane::Content,
            detail_sidebar_scroll: 0,
            detail_wikilinks: Vec::new(),
            detail_wikilink_idx: None,

            graph_page: None,
            graph_depth: 1,
            graph_outgoing: Vec::new(),
            graph_incoming: Vec::new(),
            graph_pane: LinkPane::Forward,
            graph_out_selected: 0,
            graph_in_selected: 0,

            switcher_open: false,
            switcher_input: Input::default(),
            switcher_selected: 0,

            history: Vec::new(),

            timeline_snapshots: Vec::new(),
            timeline_idx: None,
            historical_loader: None,
            live_snapshot: None,
        }
    }

    /// Install a timeline loader and snapshot list (called from cmd_tui after
    /// the history backend is opened; a no-op when the history feature is off).
    pub fn set_timeline(&mut self, snapshots: Vec<SnapshotLabel>, loader: HistoricalLoader) {
        self.timeline_snapshots = snapshots;
        self.historical_loader = Some(loader);
    }

    // ── Timeline helpers ──────────────────────────────────────────────────

    /// Load the snapshot at `idx`, storing live data on first entry.
    /// Returns `true` on success (loader returned data), `false` if unavailable.
    fn enter_snapshot(&mut self, idx: usize) -> bool {
        if idx >= self.timeline_snapshots.len() {
            return false;
        }
        let result = match self.historical_loader.as_ref() {
            Some(loader) => loader(idx),
            None => return false,
        };
        match result {
            Ok((new_files, new_graph)) => {
                let already_historical = self.timeline_idx.is_some();
                let old_files = std::mem::replace(&mut self.files, new_files);
                let old_graph = std::mem::replace(&mut self.graph, new_graph);
                if !already_historical {
                    self.live_snapshot = Some((old_files, old_graph));
                }
                self.timeline_idx = Some(idx);
                self.reload_derived_data();
                true
            }
            Err(_) => false,
        }
    }

    /// Return to live view, restoring the data saved on first history entry.
    pub fn return_to_live(&mut self) {
        if let Some((files, graph)) = self.live_snapshot.take() {
            self.files = files;
            self.graph = graph;
            self.reload_derived_data();
        }
        self.timeline_idx = None;
    }

    /// Move one snapshot older (higher index = further back in time).
    pub fn go_prev_snapshot(&mut self) {
        if self.timeline_snapshots.is_empty() {
            return;
        }
        let new_idx = match self.timeline_idx {
            None => 0, // live → most recent snapshot
            Some(i) if i + 1 < self.timeline_snapshots.len() => i + 1,
            Some(i) => i, // already at oldest, stay
        };
        if self.timeline_idx != Some(new_idx) {
            self.enter_snapshot(new_idx);
        }
    }

    /// Move one snapshot newer (lower index = closer to now); live when at index 0.
    pub fn go_next_snapshot(&mut self) {
        match self.timeline_idx {
            None => {} // already live
            Some(0) => self.return_to_live(),
            Some(i) => {
                self.enter_snapshot(i - 1);
            }
        }
    }

    /// Jump to the most recent snapshot whose calendar day is one day before
    /// the current snapshot (or before today when viewing live).
    pub fn go_prev_day(&mut self) {
        if self.timeline_snapshots.is_empty() {
            return;
        }
        let current_day = match self.timeline_idx {
            Some(i) => self.timeline_snapshots[i].timestamp_unix / 86400,
            None => {
                // Use system wall-clock for "today"
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64 / 86400)
                    .unwrap_or(0)
            }
        };
        let target_day = current_day - 1;
        // Snapshots are newest-first; the first one with day ≤ target is the
        // most recent snapshot on target_day (or earlier).
        if let Some(i) = self
            .timeline_snapshots
            .iter()
            .position(|s| s.timestamp_unix / 86400 <= target_day)
        {
            self.enter_snapshot(i);
        }
    }

    /// Jump to the most recent snapshot whose calendar day is one day after
    /// the current snapshot; returns to live when no newer day exists.
    pub fn go_next_day(&mut self) {
        let Some(current_idx) = self.timeline_idx else {
            return; // already live
        };
        let current_day = self.timeline_snapshots[current_idx].timestamp_unix / 86400;
        let target_day = current_day + 1;
        // Find the most recent snapshot (lowest index) with day ≥ target_day.
        match self
            .timeline_snapshots
            .iter()
            .position(|s| s.timestamp_unix / 86400 >= target_day)
        {
            Some(i) => {
                self.enter_snapshot(i);
            }
            None => self.return_to_live(),
        }
    }

    /// Recompute page_names, dead_links, orphans, diagnostics after a data swap.
    fn reload_derived_data(&mut self) {
        let mut page_names: Vec<String> = self.files.iter().map(|f| f.page_name.clone()).collect();
        page_names.sort_by_key(|a| a.to_lowercase());
        self.page_names = page_names;
        self.dead_links = self.graph.dead_links();
        self.orphans = self.graph.orphans();
        self.diagnostics = self
            .files
            .iter()
            .flat_map(|f| f.diagnostics.clone())
            .collect();
        // Reset list selections so stale indices don't panic
        self.page_selected = 0;
        self.page_scroll_offset = 0;
        self.diag_selected = 0;
        self.search_results.clear();
        self.search_selected = 0;
    }

    pub fn filtered_pages(&self) -> Vec<&str> {
        let filter = self.page_filter.value().to_lowercase();
        if filter.is_empty() {
            self.page_names.iter().map(|s| s.as_str()).collect()
        } else {
            self.page_names
                .iter()
                .filter(|p| p.to_lowercase().contains(&filter))
                .map(|s| s.as_str())
                .collect()
        }
    }

    pub fn switcher_filtered(&self) -> Vec<&str> {
        let filter = self.switcher_input.value().to_lowercase();
        if filter.is_empty() {
            self.page_names.iter().map(|s| s.as_str()).collect()
        } else {
            self.page_names
                .iter()
                .filter(|p| p.to_lowercase().contains(&filter))
                .map(|s| s.as_str())
                .collect()
        }
    }

    fn read_line_context(&self, page: &str, line: u32) -> Option<String> {
        let file = self.files.iter().find(|f| f.page_name == page)?;
        let full_path = self.vault_root.join(&file.path);
        let content = std::fs::read_to_string(&full_path).ok()?;
        let line_str = content.lines().nth((line as usize).saturating_sub(1))?;
        let trimmed = line_str.trim();
        if trimmed.len() > 80 {
            Some(format!("{}...", &trimmed[..77]))
        } else {
            Some(trimmed.to_string())
        }
    }

    fn select_link_page(&mut self, page: &str) {
        self.link_selected_page = Some(page.to_string());

        // Forward links with context
        let fwd = self.graph.forward_links(page);
        self.link_fwd = fwd
            .iter()
            .map(|f| {
                let ctx = self.read_line_context(page, f.meta.line);
                LinkEntry {
                    page: f.target.clone(),
                    line: f.meta.line,
                    context: ctx,
                }
            })
            .collect();

        // Backlinks with context
        let back = self.graph.backlinks(page);
        self.link_back = back
            .iter()
            .map(|b| {
                let ctx = self.read_line_context(&b.source, b.line);
                LinkEntry {
                    page: b.source.clone(),
                    line: b.line,
                    context: ctx,
                }
            })
            .collect();

        self.link_fwd_selected = 0;
        self.link_back_selected = 0;
    }

    /// Navigate to a page in the Page Detail tab.
    pub fn navigate_to_page(&mut self, page: &str) {
        // Push current page to history before navigating
        if let Some(ref current) = self.detail_page {
            self.history.push(current.clone());
            // Keep history bounded
            if self.history.len() > 50 {
                self.history.remove(0);
            }
        }
        self.load_page_detail(page);
        self.active_tab = Tab::PageDetail;
    }

    fn load_page_detail(&mut self, page: &str) {
        self.detail_page = Some(page.to_string());
        self.detail_scroll = 0;
        self.detail_sidebar_scroll = 0;
        self.detail_pane = DetailPane::Content;
        self.detail_wikilink_idx = None;

        // Read and render content
        if let Some(file) = self.files.iter().find(|f| f.page_name == page) {
            let full_path = self.vault_root.join(&file.path);
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                self.detail_frontmatter = markdown::parse_frontmatter(&content);
                self.detail_content = markdown::render_markdown(&content);
            } else {
                self.detail_content = vec![Line::raw("  (could not read file)")];
                self.detail_frontmatter = Vec::new();
            }
        } else {
            self.detail_content = vec![Line::raw("  (page not found — phantom link target)")];
            self.detail_frontmatter = Vec::new();
        }

        // Extract wikilinks from rendered content
        self.detail_wikilinks = extract_wikilinks_from_lines(&self.detail_content);

        // Backlinks with context
        let back = self.graph.backlinks(page);
        self.detail_backlinks = back
            .iter()
            .map(|b| {
                let ctx = self.read_line_context(&b.source, b.line);
                LinkEntry {
                    page: b.source.clone(),
                    line: b.line,
                    context: ctx,
                }
            })
            .collect();

        // Unlinked mentions
        self.detail_unlinked = self.find_unlinked_mentions(page);
    }

    fn find_unlinked_mentions(&self, page: &str) -> Vec<UnlinkedMention> {
        let page_lower = page.to_lowercase();
        let mut mentions = Vec::new();

        // Collect pages that formally link to this page
        let formal_linkers: HashSet<String> = self
            .graph
            .backlinks(page)
            .iter()
            .map(|b| b.source.clone())
            .collect();

        for file in &self.files {
            // Skip the page itself
            if file.page_name == page {
                continue;
            }
            // Skip pages that already formally link
            if formal_linkers.contains(&file.page_name) {
                continue;
            }

            let full_path = self.vault_root.join(&file.path);
            let content = match std::fs::read_to_string(&full_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for (line_idx, line) in content.lines().enumerate() {
                let line_lower = line.to_lowercase();
                if line_lower.contains(&page_lower) {
                    // Make sure it's not inside a [[ ]] wikilink
                    if let Some(pos) = line_lower.find(&page_lower) {
                        let before = &line[..pos];
                        let after = &line[pos + page.len()..];
                        let inside_link = before
                            .rfind("[[")
                            .is_some_and(|start| !before[start..].contains("]]"))
                            || after.starts_with("]]");
                        if inside_link {
                            continue;
                        }
                    }

                    let trimmed = line.trim();
                    let ctx = if trimmed.len() > 80 {
                        format!("{}...", &trimmed[..77])
                    } else {
                        trimmed.to_string()
                    };

                    mentions.push(UnlinkedMention {
                        page: file.page_name.clone(),
                        line: (line_idx + 1) as u32,
                        context: ctx,
                    });

                    // Only one mention per file
                    break;
                }
            }

            if mentions.len() >= 20 {
                break;
            }
        }

        mentions
    }

    /// Load local graph data for a page.
    pub fn load_graph_page(&mut self, page: &str) {
        // Push current page to history
        if let Some(ref current) = self.graph_page {
            if !self.history.contains(current) || self.history.last() != Some(current) {
                self.history.push(current.clone());
                if self.history.len() > 50 {
                    self.history.remove(0);
                }
            }
        }

        self.graph_page = Some(page.to_string());
        self.graph_out_selected = 0;
        self.graph_in_selected = 0;
        self.rebuild_graph_neighbors();
    }

    fn rebuild_graph_neighbors(&mut self) {
        let page = match &self.graph_page {
            Some(p) => p.clone(),
            None => return,
        };

        // Outgoing (forward links)
        let fwd = self.graph.forward_links(&page);
        self.graph_outgoing = fwd
            .iter()
            .map(|f| {
                let children = if self.graph_depth >= 2 {
                    let hop2 = self.graph.forward_links(&f.target);
                    hop2.iter()
                        .filter(|h| h.target != page)
                        .map(|h| (h.target.clone(), 1))
                        .collect()
                } else {
                    Vec::new()
                };
                GraphNeighbor {
                    page: f.target.clone(),
                    link_count: 1,
                    children,
                }
            })
            .collect();

        // Incoming (backlinks)
        let back = self.graph.backlinks(&page);
        self.graph_incoming = back
            .iter()
            .map(|b| {
                let children = if self.graph_depth >= 2 {
                    let hop2 = self.graph.backlinks(&b.source);
                    hop2.iter()
                        .filter(|h| h.source != page)
                        .map(|h| (h.source.clone(), 1))
                        .collect()
                } else {
                    Vec::new()
                };
                GraphNeighbor {
                    page: b.source.clone(),
                    link_count: 1,
                    children,
                }
            })
            .collect();
    }

    fn run_search(&mut self) {
        let query = self.search_input.value().to_string();
        if query.is_empty() {
            self.search_results.clear();
            return;
        }

        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for file in &self.files {
            let full_path = self.vault_root.join(&file.path);
            let content = match std::fs::read_to_string(&full_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for (line_idx, line) in content.lines().enumerate() {
                if let Some(col) = line.to_lowercase().find(&query_lower) {
                    let ctx_start = col.saturating_sub(30);
                    let ctx_end = (col + query.len() + 30).min(line.len());
                    let context = line.get(ctx_start..ctx_end).map(|s| s.to_string());

                    results.push(SearchResult {
                        page: file.page_name.clone(),
                        path: file.path.to_string_lossy().to_string(),
                        line: (line_idx + 1) as u32,
                        column: (col + 1) as u32,
                        context,
                    });

                    if results.len() >= 100 {
                        break;
                    }
                }
            }
            if results.len() >= 100 {
                break;
            }
        }

        self.search_results = results;
        self.search_selected = 0;
    }

    fn go_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            match self.active_tab {
                Tab::PageDetail => {
                    self.load_page_detail(&prev);
                }
                Tab::Graph => {
                    self.graph_page = Some(prev.clone());
                    self.rebuild_graph_neighbors();
                }
                _ => {}
            }
        }
    }
}

/// Scan rendered Lines for [[wikilinks]] and return their positions and targets.
fn extract_wikilinks_from_lines(lines: &[Line]) -> Vec<ContentWikilink> {
    let re = Regex::new(r"\[\[([^\[\]]+)\]\]").unwrap();
    let mut results = Vec::new();
    for (line_idx, line) in lines.iter().enumerate() {
        let text: String = line.iter().map(|s| s.content.as_ref()).collect();
        let mut match_on_line = 0;
        for cap in re.captures_iter(&text) {
            let raw = cap.get(1).unwrap().as_str();
            // Parse: take part before | (alias) and before # (heading)
            let target = raw.split('|').next().unwrap_or(raw);
            let target = target.split('#').next().unwrap_or(target).trim();
            if !target.is_empty() {
                results.push(ContentWikilink {
                    rendered_line: line_idx,
                    target: target.to_string(),
                    match_on_line,
                });
                match_on_line += 1;
            }
        }
    }
    results
}

// ── Terminal management ───────────────────────────────────────────────────

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

// ── Main run loop ─────────────────────────────────────────────────────────

pub fn run(app: &mut App) -> Result<()> {
    let mut terminal = setup_terminal()?;

    let result = run_loop(&mut terminal, app);

    restore_terminal()?;
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key(app, key.code, key.modifiers);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────

fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    // Global: Ctrl+C always quits
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }

    // Quick switcher overlay handling
    if app.switcher_open {
        handle_switcher_key(app, code, modifiers);
        return;
    }

    // Global: Ctrl+K opens quick switcher (works everywhere)
    if code == KeyCode::Char('k') && modifiers.contains(KeyModifiers::CONTROL) {
        app.switcher_open = true;
        app.switcher_input = Input::default();
        app.switcher_selected = 0;
        return;
    }

    if app.show_help {
        app.show_help = false;
        return;
    }

    if code == KeyCode::Char('?') && !is_input_mode(app) {
        app.show_help = true;
        return;
    }

    // Tab switching (works unless typing in an input)
    if !is_input_mode(app) {
        match code {
            KeyCode::Char('q') => {
                app.should_quit = true;
                return;
            }
            KeyCode::Tab => {
                let idx = app.active_tab.index();
                let next = (idx + 1) % Tab::ALL.len();
                app.active_tab = Tab::ALL[next];
                return;
            }
            KeyCode::BackTab => {
                let idx = app.active_tab.index();
                let prev = if idx == 0 {
                    Tab::ALL.len() - 1
                } else {
                    idx - 1
                };
                app.active_tab = Tab::ALL[prev];
                return;
            }
            // ── Timeline navigation (REQ-091) ────────────────────────────
            // [ / ] → prev/next snapshot; { / } → jump by day; n → live
            KeyCode::Char('[') if !app.timeline_snapshots.is_empty() => {
                app.go_prev_snapshot();
                return;
            }
            KeyCode::Char(']') if !app.timeline_snapshots.is_empty() => {
                app.go_next_snapshot();
                return;
            }
            KeyCode::Char('{') if !app.timeline_snapshots.is_empty() => {
                app.go_prev_day();
                return;
            }
            KeyCode::Char('}') if !app.timeline_snapshots.is_empty() => {
                app.go_next_day();
                return;
            }
            KeyCode::Char('n') if app.timeline_idx.is_some() => {
                app.return_to_live();
                return;
            }
            // Esc returns to live when in historical mode
            KeyCode::Esc if app.timeline_idx.is_some() => {
                app.return_to_live();
                return;
            }
            _ => {}
        }
    }

    // Tab-specific handlers
    match app.active_tab {
        Tab::Dashboard => {}
        Tab::Pages => handle_pages_key(app, code, modifiers),
        Tab::Links => handle_links_key(app, code, modifiers),
        Tab::Search => handle_search_key(app, code, modifiers),
        Tab::Diagnostics => handle_diagnostics_key(app, code),
        Tab::PageDetail => handle_detail_key(app, code, modifiers),
        Tab::Graph => handle_graph_key(app, code),
    }
}

fn is_input_mode(app: &App) -> bool {
    match app.active_tab {
        Tab::Pages => true,
        Tab::Links => app.link_selected_page.is_none(),
        Tab::Search => app.search_active,
        _ => false,
    }
}

fn handle_switcher_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Esc => {
            app.switcher_open = false;
        }
        KeyCode::Enter => {
            let pages = app.switcher_filtered();
            if let Some(&page) = pages.get(app.switcher_selected) {
                let page = page.to_string();
                app.switcher_open = false;
                app.navigate_to_page(&page);
            }
        }
        KeyCode::Up => {
            if app.switcher_selected > 0 {
                app.switcher_selected -= 1;
            }
        }
        KeyCode::Down => {
            let max = app.switcher_filtered().len().saturating_sub(1);
            if app.switcher_selected < max {
                app.switcher_selected += 1;
            }
        }
        _ => {
            let prev = app.switcher_input.value().to_string();
            app.switcher_input
                .handle_event(&Event::Key(event::KeyEvent::new(code, modifiers)));
            if app.switcher_input.value() != prev {
                app.switcher_selected = 0;
            }
        }
    }
}

fn handle_pages_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Up | KeyCode::Char('k') if modifiers.is_empty() || code == KeyCode::Up => {
            if app.page_selected > 0 {
                app.page_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') if modifiers.is_empty() || code == KeyCode::Down => {
            let max = app.filtered_pages().len().saturating_sub(1);
            if app.page_selected < max {
                app.page_selected += 1;
            }
        }
        KeyCode::Enter => {
            let pages = app.filtered_pages();
            if let Some(&page) = pages.get(app.page_selected) {
                let page = page.to_string();
                app.navigate_to_page(&page);
            }
        }
        KeyCode::Esc => {
            app.page_filter = Input::default();
            app.page_selected = 0;
        }
        _ => {
            let prev_val = app.page_filter.value().to_string();
            app.page_filter
                .handle_event(&Event::Key(event::KeyEvent::new(code, modifiers)));
            if app.page_filter.value() != prev_val {
                app.page_selected = 0;
            }
        }
    }
}

fn handle_links_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.link_selected_page.is_none() {
        match code {
            KeyCode::Enter => {
                let input = app.link_page_input.value().to_string();
                if !input.is_empty() {
                    let found = app
                        .page_names
                        .iter()
                        .find(|p| p.to_lowercase() == input.to_lowercase())
                        .cloned();
                    if let Some(page) = found {
                        app.select_link_page(&page);
                    }
                }
            }
            KeyCode::Esc => {
                app.link_page_input = Input::default();
            }
            _ => {
                app.link_page_input
                    .handle_event(&Event::Key(event::KeyEvent::new(code, modifiers)));
            }
        }
        return;
    }

    match code {
        KeyCode::Esc => {
            app.link_selected_page = None;
            app.link_fwd.clear();
            app.link_back.clear();
            app.link_page_input = Input::default();
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.link_pane = LinkPane::Forward;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.link_pane = LinkPane::Backward;
        }
        KeyCode::Up | KeyCode::Char('k') => match app.link_pane {
            LinkPane::Forward => {
                if app.link_fwd_selected > 0 {
                    app.link_fwd_selected -= 1;
                }
            }
            LinkPane::Backward => {
                if app.link_back_selected > 0 {
                    app.link_back_selected -= 1;
                }
            }
        },
        KeyCode::Down | KeyCode::Char('j') => match app.link_pane {
            LinkPane::Forward => {
                let max = app.link_fwd.len().saturating_sub(1);
                if app.link_fwd_selected < max {
                    app.link_fwd_selected += 1;
                }
            }
            LinkPane::Backward => {
                let max = app.link_back.len().saturating_sub(1);
                if app.link_back_selected < max {
                    app.link_back_selected += 1;
                }
            }
        },
        KeyCode::Enter => {
            let page = match app.link_pane {
                LinkPane::Forward => app
                    .link_fwd
                    .get(app.link_fwd_selected)
                    .map(|f| f.page.clone()),
                LinkPane::Backward => app
                    .link_back
                    .get(app.link_back_selected)
                    .map(|b| b.page.clone()),
            };
            if let Some(page) = page {
                app.navigate_to_page(&page);
            }
        }
        // 'p' opens page detail for current link page
        KeyCode::Char('p') => {
            if let Some(page) = app.link_selected_page.clone() {
                app.navigate_to_page(&page);
            }
        }
        // 'g' opens graph for current link page
        KeyCode::Char('g') => {
            if let Some(page) = app.link_selected_page.clone() {
                app.load_graph_page(&page);
                app.active_tab = Tab::Graph;
            }
        }
        _ => {}
    }
}

fn handle_search_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.search_active {
        match code {
            KeyCode::Enter => {
                app.run_search();
                app.search_active = false;
            }
            KeyCode::Esc => {
                app.search_active = false;
            }
            _ => {
                app.search_input
                    .handle_event(&Event::Key(event::KeyEvent::new(code, modifiers)));
            }
        }
    } else {
        match code {
            KeyCode::Char('/') | KeyCode::Char('i') => {
                app.search_active = true;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.search_selected > 0 {
                    app.search_selected -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = app.search_results.len().saturating_sub(1);
                if app.search_selected < max {
                    app.search_selected += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(result) = app.search_results.get(app.search_selected) {
                    let page = result.page.clone();
                    app.navigate_to_page(&page);
                }
            }
            _ => {}
        }
    }
}

fn handle_diagnostics_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Left | KeyCode::Char('h') => {
            app.diag_category = app.diag_category.prev();
            app.diag_selected = 0;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.diag_category = app.diag_category.next();
            app.diag_selected = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.diag_selected > 0 {
                app.diag_selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = match app.diag_category {
                DiagCategory::DeadLinks => app.dead_links.len(),
                DiagCategory::Orphans => app.orphans.len(),
                DiagCategory::Syntax => app.diagnostics.len(),
            }
            .saturating_sub(1);
            if app.diag_selected < max {
                app.diag_selected += 1;
            }
        }
        _ => {}
    }
}

fn handle_detail_key(app: &mut App, code: KeyCode, _modifiers: KeyModifiers) {
    match code {
        KeyCode::Backspace => {
            app.go_back();
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.detail_pane = DetailPane::Content;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.detail_pane = DetailPane::Sidebar;
        }
        KeyCode::Up | KeyCode::Char('k') => match app.detail_pane {
            DetailPane::Content => {
                // Jump to previous wikilink
                if app.detail_wikilinks.is_empty() {
                    app.detail_scroll = app.detail_scroll.saturating_sub(1);
                } else {
                    let new_idx = match app.detail_wikilink_idx {
                        Some(idx) if idx > 0 => idx - 1,
                        Some(_) => 0,
                        None => app.detail_wikilinks.len().saturating_sub(1),
                    };
                    app.detail_wikilink_idx = Some(new_idx);
                    detail_scroll_to_wikilink(app);
                }
            }
            DetailPane::Sidebar => {
                if app.detail_sidebar_scroll > 0 {
                    app.detail_sidebar_scroll -= 1;
                }
            }
        },
        KeyCode::Down | KeyCode::Char('j') => match app.detail_pane {
            DetailPane::Content => {
                // Jump to next wikilink
                if app.detail_wikilinks.is_empty() {
                    let max = app.detail_content.len().saturating_sub(1);
                    if app.detail_scroll < max {
                        app.detail_scroll += 1;
                    }
                } else {
                    let new_idx = match app.detail_wikilink_idx {
                        Some(idx) if idx + 1 < app.detail_wikilinks.len() => idx + 1,
                        Some(idx) => idx,
                        None => 0,
                    };
                    app.detail_wikilink_idx = Some(new_idx);
                    detail_scroll_to_wikilink(app);
                }
            }
            DetailPane::Sidebar => {
                let max = app.detail_backlinks.len() + app.detail_unlinked.len();
                if app.detail_sidebar_scroll < max.saturating_sub(1) {
                    app.detail_sidebar_scroll += 1;
                }
            }
        },
        KeyCode::Enter => match app.detail_pane {
            DetailPane::Content => {
                // Navigate to selected wikilink
                if let Some(idx) = app.detail_wikilink_idx {
                    if let Some(wl) = app.detail_wikilinks.get(idx) {
                        let target = wl.target.clone();
                        app.navigate_to_page(&target);
                    }
                }
            }
            DetailPane::Sidebar => {
                let bl_count = app.detail_backlinks.len();
                let idx = app.detail_sidebar_scroll;
                let page = if idx < bl_count {
                    Some(app.detail_backlinks[idx].page.clone())
                } else {
                    let unlinked_idx = idx - bl_count;
                    app.detail_unlinked
                        .get(unlinked_idx)
                        .map(|u| u.page.clone())
                };
                if let Some(page) = page {
                    app.navigate_to_page(&page);
                }
            }
        },
        // 'g' opens graph for this page
        KeyCode::Char('g') => {
            if let Some(page) = app.detail_page.clone() {
                app.load_graph_page(&page);
                app.active_tab = Tab::Graph;
            }
        }
        KeyCode::PageUp => {
            app.detail_scroll = app.detail_scroll.saturating_sub(20);
            app.detail_wikilink_idx = None;
        }
        KeyCode::PageDown => {
            let max = app.detail_content.len().saturating_sub(1);
            app.detail_scroll = (app.detail_scroll + 20).min(max);
            app.detail_wikilink_idx = None;
        }
        _ => {}
    }
}

/// Auto-scroll to keep the selected wikilink visible.
fn detail_scroll_to_wikilink(app: &mut App) {
    if let Some(idx) = app.detail_wikilink_idx {
        if let Some(wl) = app.detail_wikilinks.get(idx) {
            let target_line = wl.rendered_line;
            // Keep 3 lines of padding from edges
            if target_line < app.detail_scroll + 3 {
                app.detail_scroll = target_line.saturating_sub(3);
            } else if target_line > app.detail_scroll + 15 {
                // Approximate: scroll so the link is near the top third
                app.detail_scroll = target_line.saturating_sub(5);
            }
        }
    }
}

fn handle_graph_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Backspace => {
            app.go_back();
        }
        KeyCode::Char('d') => {
            // Toggle depth 1 <-> 2
            app.graph_depth = if app.graph_depth == 1 { 2 } else { 1 };
            app.rebuild_graph_neighbors();
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.graph_pane = LinkPane::Forward;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.graph_pane = LinkPane::Backward;
        }
        KeyCode::Up | KeyCode::Char('k') => match app.graph_pane {
            LinkPane::Forward => {
                if app.graph_out_selected > 0 {
                    app.graph_out_selected -= 1;
                }
            }
            LinkPane::Backward => {
                if app.graph_in_selected > 0 {
                    app.graph_in_selected -= 1;
                }
            }
        },
        KeyCode::Down | KeyCode::Char('j') => match app.graph_pane {
            LinkPane::Forward => {
                let max = app.graph_outgoing.len().saturating_sub(1);
                if app.graph_out_selected < max {
                    app.graph_out_selected += 1;
                }
            }
            LinkPane::Backward => {
                let max = app.graph_incoming.len().saturating_sub(1);
                if app.graph_in_selected < max {
                    app.graph_in_selected += 1;
                }
            }
        },
        KeyCode::Enter => {
            let page = match app.graph_pane {
                LinkPane::Forward => app
                    .graph_outgoing
                    .get(app.graph_out_selected)
                    .map(|n| n.page.clone()),
                LinkPane::Backward => app
                    .graph_incoming
                    .get(app.graph_in_selected)
                    .map(|n| n.page.clone()),
            };
            if let Some(page) = page {
                app.load_graph_page(&page);
            }
        }
        // 'p' opens page detail for the current graph center
        KeyCode::Char('p') => {
            if let Some(page) = app.graph_page.clone() {
                app.navigate_to_page(&page);
            }
        }
        _ => {}
    }
}
