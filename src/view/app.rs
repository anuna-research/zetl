use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use super::color::{detect_color_mode, ColorMode, LinkColors};
use super::link_map::{build_annotated_lines, LinkEntry, LinkMap};
use super::terminal::{enter_alternate_screen, restore_terminal};
use crate::view::event::run_event_loop;

/// Terminal width below which the view switches to single-pane mode (REQ-063).
const SINGLE_PANE_THRESHOLD: u16 = 60;

/// Width of the bridge column separating the two panes (NFR-025).
const BRIDGE_WIDTH: u16 = 3;

/// Maximum navigation history depth (REQ-069).
const MAX_HISTORY_DEPTH: usize = 50;

// ── FocusState ────────────────────────────────────────────────────────────

/// Whether the user is scrolling freely or has focused a specific wikilink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusState {
    /// Free-scroll mode — j/k scroll the main pane.
    #[default]
    ScrollMode,
    /// A specific visible wikilink is focused; j/k cycle through links.
    FocusMode {
        /// 0-based index into the list of currently-visible wikilinks.
        focused_index: usize,
    },
}

// ── CardData ──────────────────────────────────────────────────────────────

/// Pre-computed data for a single context card, built before rendering.
///
/// Using a separate struct avoids aliasing issues when the excerpt cache is
/// populated while the link map is borrowed.
#[derive(Debug, Clone)]
struct CardData {
    /// 1-based link ordinal (matches anchor glyph and bridge connector).
    ordinal: usize,
    page_title: String,
    is_dead: bool,
    /// Palette color for header and bridge connector (Color::Reset in no-color mode).
    color: Color,
    /// Pre-loaded excerpt lines (empty for dead links).
    excerpt: Vec<String>,
    /// Total terminal rows this card occupies.
    card_height: u16,
    is_focused: bool,
}

// ── PickerState ───────────────────────────────────────────────────────────

/// Interactive page picker overlay state (REQ-062).
pub struct PickerState {
    /// Current filter text typed by the user.
    pub query: String,
    /// 0-based index of the highlighted row in the filtered list.
    pub selected: usize,
    /// Scroll offset within the filtered list.
    pub list_scroll: usize,
}

impl PickerState {
    fn new() -> Self {
        PickerState {
            query: String::new(),
            selected: 0,
            list_scroll: 0,
        }
    }

    /// Return page titles that match the current query (case-insensitive substring).
    pub fn filtered<'a>(&self, file_index: &'a [(String, PathBuf)]) -> Vec<&'a str> {
        let q = self.query.to_lowercase();
        file_index
            .iter()
            .filter(|(name, _)| q.is_empty() || name.to_lowercase().contains(&q))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Clamp `selected` and `list_scroll` to valid ranges.
    fn clamp(&mut self, file_index: &[(String, PathBuf)], list_height: usize) {
        let count = self.filtered(file_index).len();
        if count == 0 {
            self.selected = 0;
            self.list_scroll = 0;
            return;
        }
        self.selected = self.selected.min(count - 1);
        // Ensure selected is in the visible window.
        if self.selected < self.list_scroll {
            self.list_scroll = self.selected;
        } else if self.selected >= self.list_scroll + list_height {
            self.list_scroll = self.selected + 1 - list_height;
        }
    }
}

// ── ViewApp ───────────────────────────────────────────────────────────────

/// Application state for `zetl view`.
///
/// This struct is distinct from [`crate::tui::App`]: it drives the
/// Xanadu-style two-pane view (SPEC-009) rather than the dashboard TUI.
pub struct ViewApp {
    /// Title of the page currently displayed in the main pane.
    pub current_page: String,

    /// Raw Markdown lines of the current page.  Empty when no file is loaded.
    pub content_lines: Vec<String>,

    /// Pre-built ratatui [`Line`]s with `[N]` anchor glyphs injected after
    /// each `[[wikilink]]` (REQ-064).  Parallel to `content_lines`.
    pub annotated_lines: Vec<Line<'static>>,

    /// Document-order list of all wikilinks found in the current page (REQ-064).
    pub link_map: LinkMap,

    /// Terminal color capability detected at startup.
    pub color_mode: ColorMode,

    /// Number of lines scrolled down from the top of the document.
    pub scroll_offset: usize,

    /// Height (in terminal rows) of the scrollable content area — refreshed every draw call.
    pub viewport_height: usize,

    /// Navigation history as `(page_title, scroll_offset)` pairs (REQ-069).
    pub nav_history: Vec<(String, usize)>,

    /// Forward navigation stack for `]` key (REQ-069).
    pub forward_history: Vec<(String, usize)>,

    /// Vault root used to resolve relative paths from `file_index`.
    pub vault_root: PathBuf,

    /// Mapping of page title → relative path for every page in the vault.
    pub file_index: Vec<(String, PathBuf)>,

    /// Complete set of vault page titles for dead-link detection.
    pub page_set: HashSet<String>,

    /// Whether a wikilink is focused for navigation.
    pub focus_state: FocusState,

    /// Lines shown per context card in non-focused mode (1–20, default 5).
    pub context_lines: u8,

    /// Percentage of terminal columns allocated to the main pane (30–80, default 58).
    pub main_width: u8,

    /// When `true`, the keybindings help overlay is visible.
    pub show_help: bool,

    /// When `true`, the context pane is shown as a full-screen overlay
    /// (single-pane fallback mode toggled by Ctrl-R).
    pub context_overlay: bool,

    /// When `true`, the interactive page picker overlay is visible.
    pub show_picker: bool,

    /// Page picker state when `show_picker` is `true`.
    pub picker_state: Option<PickerState>,

    /// Set to `true` to break out of the event loop.
    pub should_quit: bool,

    // ── Backlinks ─────────────────────────────────────────────────────────
    /// Backlink data: target_page_title → [(citing_page_title, line_number)].
    /// Populated at startup from the link graph; used in Back/Both context mode.
    pub backlink_map: HashMap<String, Vec<(String, u32)>>,

    // ── Excerpt cache ─────────────────────────────────────────────────────
    /// Cache of page excerpts: page_title → first 20 lines.
    pub excerpt_cache: HashMap<String, Vec<String>>,

    // ── Bridge rendering positions ────────────────────────────────────────
    /// Anchor glyph terminal rows: ordinal → absolute terminal row.
    /// Populated in `draw_main_pane`, used by `draw_bridge_col`.
    pub anchor_rows: HashMap<usize, u16>,

    /// Card header terminal rows: ordinal → absolute terminal row.
    /// Populated in `draw_context_pane`, used by `draw_bridge_col`.
    pub card_rows: HashMap<usize, u16>,

    // ── Transient status ──────────────────────────────────────────────────
    /// Transient status bar message (e.g., "dead link" warning).
    pub status_message: Option<String>,

    // ── Debug render (OBS-012) ────────────────────────────────────────────
    /// Application start time for ZETL_DEBUG_RENDER timing.
    pub start_time: Instant,

    /// Whether ZETL_DEBUG_RENDER=1 is set.
    pub debug_render: bool,

    /// Count of distinct pages visited (for quit timing line).
    pub pages_visited: usize,

    // ── Computed layout rects (refreshed every draw call) ─────────────────
    /// Rect of the main note pane (left column).
    pub main_pane: Rect,

    /// Rect of the 3-column bridge strip between the two panes.
    pub bridge_col: Rect,

    /// Rect of the context pane (right column).  Empty in single-pane mode.
    pub context_pane: Rect,

    /// Rect of the single-row status bar pinned to the bottom.
    pub status_bar: Rect,

    /// `true` when the terminal is narrower than [`SINGLE_PANE_THRESHOLD`].
    pub single_pane: bool,
}

impl ViewApp {
    /// Create a new [`ViewApp`] starting on `page`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        page: impl Into<String>,
        file_path: Option<PathBuf>,
        context_lines: u8,
        main_width: u8,
        page_set: HashSet<String>,
        file_index: Vec<(String, PathBuf)>,
        vault_root: PathBuf,
        backlink_map: HashMap<String, Vec<(String, u32)>>,
    ) -> Self {
        let current_page = page.into();
        let content_lines: Vec<String> = file_path
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default();

        let color_mode = detect_color_mode();
        let (annotated_lines, link_map) =
            build_annotated_lines(&content_lines, &page_set, color_mode);

        let debug_render = std::env::var_os("ZETL_DEBUG_RENDER").is_some_and(|v| v != "0");

        // Open the picker when no page was specified.
        let show_picker = current_page.is_empty() || current_page == "(no page selected)";
        let picker_state = if show_picker {
            Some(PickerState::new())
        } else {
            None
        };

        Self {
            current_page,
            content_lines,
            annotated_lines,
            link_map,
            color_mode,
            scroll_offset: 0,
            viewport_height: 0,
            nav_history: Vec::new(),
            forward_history: Vec::new(),
            vault_root,
            file_index,
            page_set,
            focus_state: FocusState::default(),
            context_lines,
            main_width,
            show_help: false,
            context_overlay: false,
            show_picker,
            picker_state,
            should_quit: false,
            backlink_map,
            excerpt_cache: HashMap::new(),
            anchor_rows: HashMap::new(),
            card_rows: HashMap::new(),
            status_message: None,
            start_time: Instant::now(),
            debug_render,
            pages_visited: if show_picker { 0 } else { 1 },
            main_pane: Rect::default(),
            bridge_col: Rect::default(),
            context_pane: Rect::default(),
            status_bar: Rect::default(),
            single_pane: false,
        }
    }

    /// Open the alternate screen, run the event loop, then restore the terminal.
    pub fn run(&mut self) -> Result<()> {
        if self.debug_render {
            let index_ms = self.start_time.elapsed().as_millis();
            eprintln!(
                "[zetl view] startup  index_load_ms={} page={}",
                index_ms, self.current_page
            );
        }
        let mut terminal = enter_alternate_screen()?;
        let result = run_event_loop(&mut terminal, self);
        restore_terminal()?;
        if self.debug_render {
            let uptime_s = self.start_time.elapsed().as_secs();
            eprintln!(
                "[zetl view] quit  uptime_s={} pages_visited={} nav_history_depth={}",
                uptime_s,
                self.pages_visited,
                self.nav_history.len(),
            );
        }
        result
    }

    /// Render a single frame into `frame`, updating stored pane rects (REQ-063, NFR-025).
    pub fn draw(&mut self, frame: &mut Frame) {
        let render_start = Instant::now();
        let area = frame.area();

        // ── Step 1: vertical split — content + 1-row status bar ──────────
        let [content_area, status_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        self.status_bar = status_area;

        // ── Step 2: horizontal split based on terminal width ─────────────
        if content_area.width < SINGLE_PANE_THRESHOLD {
            self.single_pane = true;
            self.main_pane = content_area;
            self.bridge_col = Rect::default();
            self.context_pane = Rect::default();
        } else {
            self.single_pane = false;
            let [main_area, bridge_area, context_area] = Layout::horizontal([
                Constraint::Percentage(self.main_width as u16),
                Constraint::Length(BRIDGE_WIDTH),
                Constraint::Fill(1),
            ])
            .areas(content_area);
            self.main_pane = main_area;
            self.bridge_col = bridge_area;
            self.context_pane = context_area;
        }

        // ── Step 3: render panes (order matters for bridge position data) ─
        self.draw_main_pane(frame); // populates anchor_rows

        if !self.single_pane {
            self.draw_context_pane(frame); // populates card_rows
            self.draw_bridge_col(frame); // uses anchor_rows + card_rows
        }

        // ── Step 4: status bar and overlays ──────────────────────────────
        let status_area = self.status_bar;
        self.draw_status_bar(frame, status_area);

        if self.context_overlay {
            self.draw_context_overlay(frame, area);
        }
        if self.show_help {
            self.draw_help_overlay(frame, area);
        }
        if self.show_picker {
            self.draw_picker_overlay(frame, area);
        }

        if self.debug_render {
            let total_ms = render_start.elapsed().as_millis();
            eprintln!("[zetl view] render  total_ms={total_ms}");
        }
    }

    // ── Main pane ─────────────────────────────────────────────────────────

    /// Render the main note pane and populate `anchor_rows` for bridge rendering.
    fn draw_main_pane(&mut self, frame: &mut Frame) {
        let area = self.main_pane;

        let [header_area, content_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

        self.viewport_height = content_area.height as usize;

        let max_scroll = self
            .content_lines
            .len()
            .saturating_sub(self.viewport_height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        // ── Populate anchor_rows ──────────────────────────────────────────
        self.anchor_rows.clear();
        let content_y = content_area.y;
        for entry in &self.link_map {
            if self.viewport_height > 0
                && entry.line_number >= self.scroll_offset
                && entry.line_number < self.scroll_offset + self.viewport_height
            {
                let row = content_y + (entry.line_number - self.scroll_offset) as u16;
                self.anchor_rows.insert(entry.ordinal, row);
            }
        }

        // ── Title header ──────────────────────────────────────────────────
        let header = Paragraph::new(format!(" {} ", self.current_page))
            .style(Style::default().bold().reversed());
        frame.render_widget(header, header_area);

        // ── Scrollable content ────────────────────────────────────────────
        let scroll_row = self.scroll_offset.min(u16::MAX as usize) as u16;
        let para = Paragraph::new(self.annotated_lines_with_focus())
            .wrap(Wrap { trim: false })
            .scroll((scroll_row, 0));
        frame.render_widget(para, content_area);
    }

    // ── Context pane ──────────────────────────────────────────────────────

    /// Render context cards in the right pane and populate `card_rows` (REQ-065).
    ///
    /// The pane is split: forward-link cards fill the upper portion, and a
    /// compact backlinks list is pinned to the bottom.
    fn draw_context_pane(&mut self, frame: &mut Frame) {
        let area = self.context_pane;
        if area.is_empty() {
            return;
        }
        self.card_rows.clear();

        // Compute how much vertical space the backlinks section needs.
        let back_count = self
            .backlink_map
            .get(&self.current_page.clone())
            .map(|v| v.len())
            .unwrap_or(0);
        const MAX_BACK_VISIBLE: usize = 5;
        let back_section_h: u16 = if back_count == 0 {
            0
        } else {
            let rows = back_count.min(MAX_BACK_VISIBLE);
            let overflow = usize::from(back_count > MAX_BACK_VISIBLE);
            (1 + rows + overflow) as u16 // header + entries + optional "↓ N more"
        };

        let fwd_h = area.height.saturating_sub(back_section_h);
        let fwd_area = Rect {
            height: fwd_h,
            ..area
        };
        let back_area = Rect {
            y: area.y + fwd_h,
            height: back_section_h,
            ..area
        };

        let cards = self.compute_forward_card_data(fwd_h);
        self.render_cards(frame, fwd_area, &cards);

        if back_section_h > 0 {
            self.render_backlinks_list(frame, back_area, MAX_BACK_VISIBLE);
        }
    }

    /// Render the compact backlinks section pinned to the bottom of the context pane.
    fn render_backlinks_list(&self, frame: &mut Frame, area: Rect, max_visible: usize) {
        if area.height == 0 {
            return;
        }
        let mut entries: Vec<(String, u32)> = self
            .backlink_map
            .get(&self.current_page)
            .cloned()
            .unwrap_or_default();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Section header doubles as a visual separator from the cards above.
        lines.push(Line::styled(
            format!(
                " ← {} backlink{} ",
                entries.len(),
                if entries.len() == 1 { "" } else { "s" }
            ),
            Style::default().bold().reversed(),
        ));

        for (page, line_num) in entries.iter().take(max_visible) {
            let text = if *line_num > 0 {
                format!("  ← {page}  · {line_num}")
            } else {
                format!("  ← {page}")
            };
            lines.push(Line::raw(text));
        }

        if entries.len() > max_visible {
            lines.push(Line::styled(
                format!("  ↓ {} more", entries.len() - max_visible),
                Style::default().dim(),
            ));
        }

        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Compute card data for forward-link mode.
    ///
    /// `fwd_pane_height` is the height of the forward-card area (which may be
    /// smaller than the full context pane when a backlinks section is present).
    fn compute_forward_card_data(&mut self, fwd_pane_height: u16) -> Vec<CardData> {
        let visible = self.visible_links_snapshot();
        let link_colors = LinkColors::new(self.color_mode);
        let is_no_color = self.color_mode == ColorMode::NoColor;
        let focused_vis_idx = match self.focus_state {
            FocusState::FocusMode { focused_index } => Some(focused_index),
            FocusState::ScrollMode => None,
        };

        let mut cards = Vec::new();
        for (vis_idx, entry) in visible.iter().enumerate() {
            let is_focused = focused_vis_idx == Some(vis_idx);
            // Focused card fills 80 % of the forward-card area; non-focused
            // cards stay compact at `context_lines` rows.
            let excerpt_n = if is_focused {
                let target = (fwd_pane_height as usize * 4 / 5).saturating_sub(3);
                target.max(1)
            } else {
                self.context_lines as usize
            };

            let excerpt = if entry.is_dead {
                Vec::new()
            } else {
                self.load_page_excerpt(&entry.page_title.clone(), excerpt_n)
            };

            let color = if is_no_color {
                Color::Reset
            } else if entry.is_dead {
                Color::Red
            } else {
                link_colors.get(entry.ordinal).color()
            };

            // Height: 1 (header) + excerpt_n (content) + 1 (separator).
            // Focused adds 2 for box border.
            let card_height = if is_focused {
                excerpt_n as u16 + 3 // border-top + header + excerpt + border-bottom
            } else {
                excerpt_n as u16 + 2 // header + excerpt + separator
            };

            cards.push(CardData {
                ordinal: entry.ordinal,
                page_title: entry.page_title.clone(),
                is_dead: entry.is_dead,
                color,
                excerpt,
                card_height,
                is_focused,
            });
        }
        cards
    }

    /// Render a list of forward-link cards into `area`, recording `card_rows`.
    ///
    /// When a focused card is present it is vertically centred in `area`; the
    /// cards before it fill upward and the cards after it fill downward.
    /// Without a focused card the list renders top-to-bottom.
    fn render_cards(&mut self, frame: &mut Frame, area: Rect, cards: &[CardData]) {
        let is_no_color = self.color_mode == ColorMode::NoColor;

        let Some(fi) = cards.iter().position(|c| c.is_focused) else {
            // No focused card — simple top-to-bottom layout.
            return self.render_cards_linear(frame, area, cards, is_no_color);
        };

        let focused_h = cards[fi].card_height.min(area.height);
        let center_y = area.height.saturating_sub(focused_h) / 2;

        // ── Cards above the focused card (fill upward from center_y) ─────
        let above = &cards[..fi];
        let mut y_cursor = center_y;
        let mut rendered_above = 0;
        for card in above.iter().rev() {
            if y_cursor == 0 {
                break;
            }
            let h = card.card_height.min(y_cursor);
            y_cursor = y_cursor.saturating_sub(h);
            let card_area = Rect {
                x: area.x,
                y: area.y + y_cursor,
                width: area.width,
                height: h,
            };
            self.card_rows.insert(card.ordinal, card_area.y);
            self.render_normal_card(frame, card_area, card, is_no_color);
            rendered_above += 1;
        }
        if above.len() > rendered_above && center_y > 0 {
            frame.render_widget(
                Paragraph::new(format!("↑ {} more", above.len() - rendered_above))
                    .style(Style::default().dim()),
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                },
            );
        }

        // ── Focused card (centred) ─────────────────────────────────────────
        let focused_area = Rect {
            x: area.x,
            y: area.y + center_y,
            width: area.width,
            height: focused_h,
        };
        self.card_rows.insert(cards[fi].ordinal, focused_area.y);
        self.render_focused_card(frame, focused_area, &cards[fi], is_no_color);

        // ── Cards below the focused card (fill downward) ──────────────────
        let below = &cards[fi + 1..];
        let mut y_below = center_y + focused_h;
        let mut rendered_below = 0;
        for card in below {
            let remaining = area.height.saturating_sub(y_below);
            if remaining < 2 {
                break;
            }
            let h = card.card_height.min(remaining);
            let card_area = Rect {
                x: area.x,
                y: area.y + y_below,
                width: area.width,
                height: h,
            };
            self.card_rows.insert(card.ordinal, card_area.y);
            self.render_normal_card(frame, card_area, card, is_no_color);
            y_below += h;
            rendered_below += 1;
            if y_below >= area.height {
                break;
            }
        }
        if below.len() > rendered_below && area.height > 0 {
            frame.render_widget(
                Paragraph::new(format!("↓ {} more", below.len() - rendered_below))
                    .style(Style::default().dim()),
                Rect {
                    x: area.x,
                    y: area.y + area.height - 1,
                    width: area.width,
                    height: 1,
                },
            );
        }
    }

    /// Top-to-bottom card layout used when no card is focused.
    fn render_cards_linear(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        cards: &[CardData],
        is_no_color: bool,
    ) {
        let mut y = 0u16;
        let mut rendered_count = 0;
        for card in cards {
            let remaining = area.height.saturating_sub(y);
            if remaining < 2 {
                break;
            }
            let card_h = card.card_height.min(remaining);
            let card_area = Rect {
                x: area.x,
                y: area.y + y,
                width: area.width,
                height: card_h,
            };
            self.card_rows.insert(card.ordinal, card_area.y);
            self.render_normal_card(frame, card_area, card, is_no_color);
            y += card_h;
            rendered_count += 1;
            if y >= area.height {
                break;
            }
        }
        let n_more = cards.len().saturating_sub(rendered_count);
        if n_more > 0 && area.height > 0 {
            frame.render_widget(
                Paragraph::new(format!("↓ {n_more} more")).style(Style::default().dim()),
                Rect {
                    x: area.x,
                    y: area.y + area.height - 1,
                    width: area.width,
                    height: 1,
                },
            );
        }
    }

    /// Render a single normal (non-focused) card.
    fn render_normal_card(
        &self,
        frame: &mut Frame,
        area: Rect,
        card: &CardData,
        is_no_color: bool,
    ) {
        if area.height == 0 {
            return;
        }
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Header line.
        let header_text = if card.is_dead {
            format!("[{}] {} (dead link)", card.ordinal, card.page_title)
        } else {
            format!("[{}] {}", card.ordinal, card.page_title)
        };
        let header_line = if is_no_color {
            Line::raw(header_text)
        } else {
            Line::styled(header_text, Style::default().fg(card.color).bold())
        };
        lines.push(header_line);

        // Excerpt lines (padded to context_lines).
        let excerpt_target = self.context_lines as usize;
        for line in card.excerpt.iter().take(excerpt_target) {
            lines.push(Line::raw(line.clone()));
        }
        for _ in card.excerpt.len().min(excerpt_target)..excerpt_target {
            lines.push(Line::raw(""));
        }

        // Separator.
        let sep = if is_no_color {
            "-".repeat(area.width as usize)
        } else {
            "─".repeat(area.width as usize)
        };
        lines.push(Line::raw(sep));

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }

    /// Render a focused card with expanded excerpt and box border (REQ-068).
    fn render_focused_card(
        &self,
        frame: &mut Frame,
        area: Rect,
        card: &CardData,
        is_no_color: bool,
    ) {
        if area.height < 2 {
            return;
        }
        let title = format!("[{}] {}", card.ordinal, card.page_title);
        let border_style = if is_no_color {
            Style::default()
        } else {
            Style::default().fg(card.color)
        };
        let block = Block::bordered().title(title).border_style(border_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut inner_lines: Vec<Line<'static>> = Vec::new();
        for line in &card.excerpt {
            inner_lines.push(Line::raw(line.clone()));
        }
        frame.render_widget(
            Paragraph::new(inner_lines).wrap(Wrap { trim: false }),
            inner,
        );
    }

    // ── Bridge column ─────────────────────────────────────────────────────

    /// Render bridge column connectors between anchor glyphs and card headers (REQ-066).
    fn draw_bridge_col(&self, frame: &mut Frame) {
        let area = self.bridge_col;
        if area.is_empty() {
            return;
        }

        let is_no_color = self.color_mode == ColorMode::NoColor;
        let link_colors = LinkColors::new(self.color_mode);
        let filler: &str = if is_no_color { "|||" } else { " │ " };

        // Build bridge lines starting with filler.
        let mut bridge_lines: Vec<Line<'static>> =
            vec![Line::raw(filler.to_string()); area.height as usize];

        // For each link with both an anchor and a card, place a connector.
        for (&ordinal, &anchor_row) in &self.anchor_rows {
            if let Some(&card_row) = self.card_rows.get(&ordinal) {
                let midpoint = (anchor_row as u32 + card_row as u32) / 2;
                // Convert absolute row to bridge-relative row.
                let bridge_abs_row = midpoint as u16;
                if bridge_abs_row < area.y || bridge_abs_row >= area.y + area.height {
                    continue;
                }
                let bridge_rel = bridge_abs_row - area.y;

                // Determine if this link is the currently focused one.
                let is_focused = matches!(
                    self.focus_state,
                    FocusState::FocusMode { focused_index }
                        if self.link_map
                            .get(focused_index)
                            .is_some_and(|e| e.ordinal == ordinal)
                );

                let (connector, style) = if is_no_color {
                    let c = if is_focused || anchor_row == card_row {
                        "==="
                    } else {
                        "---"
                    };
                    (c, Style::default())
                } else {
                    let c = if is_focused || anchor_row == card_row {
                        "═══"
                    } else {
                        "───"
                    };
                    let color = link_colors.get(ordinal).color();
                    (c, Style::default().fg(color))
                };

                bridge_lines[bridge_rel as usize] = Line::styled(connector.to_string(), style);
            }
        }

        frame.render_widget(Paragraph::new(bridge_lines), area);
    }

    // ── Status bar ────────────────────────────────────────────────────────

    /// Render the single-line status bar at `area`.
    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let focused_segment = match self.focus_state {
            FocusState::ScrollMode => String::new(),
            FocusState::FocusMode { focused_index } => {
                let title = self
                    .link_map
                    .get(focused_index)
                    .map(|e| e.page_title.as_str())
                    .unwrap_or("");
                format!(" │ [{}] {}", focused_index + 1, title)
            }
        };

        // Status message takes priority over the default help hint.
        let hint = if let Some(msg) = &self.status_message {
            msg.clone()
        } else {
            "j/k scroll  Tab focus  ?".to_string()
        };

        let status = format!(
            " zetl view │ {} │{} {} links │ {}",
            self.current_page,
            focused_segment,
            self.link_map.len(),
            hint,
        );

        let para = Paragraph::new(status).style(Style::default().reversed());
        frame.render_widget(para, area);
    }

    // ── Overlays ──────────────────────────────────────────────────────────

    /// Render the context-pane full-screen overlay (Ctrl-R, single-pane fallback).
    fn draw_context_overlay(&mut self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(80, 90, area);
        let block = Block::bordered().title(" Context pane ");
        let inner = block.inner(popup);
        frame.render_widget(Clear, popup);
        frame.render_widget(block, popup);

        let cards = self.compute_forward_card_data(inner.height);
        self.render_cards(frame, inner, &cards);
    }

    /// Render the keybindings help overlay (`?` key, CON-023).
    fn draw_help_overlay(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(72, 80, area);
        let block = Block::bordered().title(" Key Bindings — zetl view (CON-023) ");
        let inner = block.inner(popup);
        frame.render_widget(Clear, popup);
        frame.render_widget(block, popup);

        let help_text = concat!(
            " Scroll mode\n",
            "   j / ↓         Scroll main pane down one line\n",
            "   k / ↑         Scroll main pane up one line\n",
            "   Ctrl-d        Scroll main pane down half a page\n",
            "   Ctrl-u        Scroll main pane up half a page\n",
            "   g             Go to top of current note\n",
            "   G             Go to bottom of current note\n",
            "   Tab           Enter focused-link mode on first visible link\n",
            "\n",
            " Focused-link mode\n",
            "   j / k         Cycle focus to next/previous wikilink\n",
            "   Enter         Navigate to focused link's target page\n",
            "   Tab           Exit focused-link mode; return to scroll mode\n",
            "\n",
            " Any mode\n",
            "   b             Toggle context pane: forward → back → both\n",
            "   [             Navigate backward in session history\n",
            "   ]             Navigate forward in session history\n",
            "   Ctrl-R        Toggle context pane overlay (single-pane mode)\n",
            "   /             Open page search\n",
            "   ?             Show/hide this help overlay\n",
            "   q / Ctrl-C    Quit zetl view",
        );

        let para = Paragraph::new(help_text);
        frame.render_widget(para, inner);
    }

    /// Render the interactive page picker overlay (REQ-062).
    fn draw_picker_overlay(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(70, 80, area);
        let block = Block::bordered()
            .title(" zetl view — page search (↑↓/j/k select  Enter open  Esc cancel) ");
        let inner = block.inner(popup);
        frame.render_widget(Clear, popup);
        frame.render_widget(block, popup);

        if inner.height < 2 {
            return;
        }

        let Some(picker) = &self.picker_state else {
            return;
        };

        // Input field at the top.
        let input_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        let query_display = format!("> {}", picker.query);
        frame.render_widget(
            Paragraph::new(query_display).style(Style::default().bold()),
            input_area,
        );

        // Separator line.
        if inner.height < 3 {
            return;
        }
        let sep_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new("─".repeat(inner.width as usize)).style(Style::default().dim()),
            sep_area,
        );

        // Filtered list.
        let list_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height - 2,
        };

        let filtered = picker.filtered(&self.file_index);
        let list_height = list_area.height as usize;
        let mut lines: Vec<Line<'static>> = Vec::new();

        for (i, title) in filtered
            .iter()
            .enumerate()
            .skip(picker.list_scroll)
            .take(list_height)
        {
            let abs_idx = i + picker.list_scroll;
            let line = if abs_idx == picker.selected {
                Line::styled(format!("▶ {title}"), Style::default().bold().reversed())
            } else {
                Line::raw(format!("  {title}"))
            };
            lines.push(line);
        }

        if filtered.is_empty() {
            lines.push(Line::styled("(no matches)", Style::default().dim()));
        }

        frame.render_widget(Paragraph::new(lines), list_area);
    }

    // ── Key handling ──────────────────────────────────────────────────────

    /// Handle a key event, mutating state.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Clear one-shot status messages on any keypress.
        self.status_message = None;

        // Picker overlay intercepts all keys when active.
        if self.show_picker {
            self.handle_picker_key(code, modifiers);
            return;
        }

        // Any overlay (help or context) dismisses on Escape.
        if code == KeyCode::Esc {
            if self.show_help {
                self.show_help = false;
                return;
            }
            if self.context_overlay {
                self.context_overlay = false;
                return;
            }
        }

        // When the help overlay is open, only allow ? (toggle off) and Ctrl-C/q.
        if self.show_help {
            match code {
                KeyCode::Char('?') => {
                    self.show_help = false;
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                _ => {}
            }
            return;
        }

        // Ctrl-C always quits.
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        // Ctrl-d / Ctrl-u half-page scroll, Ctrl-R overlay toggle.
        if modifiers.contains(KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('d') => {
                    let half = (self.viewport_height / 2).max(1);
                    self.scroll_offset = self.scroll_offset.saturating_add(half);
                    return;
                }
                KeyCode::Char('u') => {
                    let half = (self.viewport_height / 2).max(1);
                    self.scroll_offset = self.scroll_offset.saturating_sub(half);
                    return;
                }
                KeyCode::Char('r') => {
                    self.context_overlay = !self.context_overlay;
                    return;
                }
                _ => {}
            }
        }

        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
            }
            KeyCode::Char('/') => {
                self.show_picker = true;
                self.picker_state = Some(PickerState::new());
            }
            KeyCode::Char('j') | KeyCode::Down => match self.focus_state {
                FocusState::FocusMode { focused_index } => {
                    let visible_len = self.visible_links_snapshot().len();
                    let new_idx = (focused_index + 1).min(visible_len.saturating_sub(1));
                    self.focus_state = FocusState::FocusMode {
                        focused_index: new_idx,
                    };
                    self.scroll_to_focused();
                }
                FocusState::ScrollMode => self.scroll_down(),
            },
            KeyCode::Char('k') | KeyCode::Up => match self.focus_state {
                FocusState::FocusMode { focused_index } => {
                    let new_idx = focused_index.saturating_sub(1);
                    self.focus_state = FocusState::FocusMode {
                        focused_index: new_idx,
                    };
                    self.scroll_to_focused();
                }
                FocusState::ScrollMode => self.scroll_up(),
            },
            KeyCode::Char('g') => {
                self.scroll_offset = 0;
            }
            KeyCode::Char('G') => {
                self.scroll_offset = self
                    .content_lines
                    .len()
                    .saturating_sub(self.viewport_height);
            }
            KeyCode::Tab => {
                self.toggle_focus();
            }
            KeyCode::Enter => {
                if let FocusState::FocusMode { focused_index } = self.focus_state {
                    let visible = self.visible_links_snapshot();
                    if let Some(entry) = visible.get(focused_index) {
                        if entry.is_dead {
                            self.status_message =
                                Some("dead link — run zetl index or create the page".to_string());
                        } else {
                            let target = entry.page_title.clone();
                            self.navigate_to(target);
                        }
                    }
                }
            }
            KeyCode::Char('[') => {
                self.navigate_back();
            }
            KeyCode::Char(']') => {
                self.navigate_forward();
            }
            _ => {}
        }
    }

    /// Handle a key event when the page picker overlay is active.
    fn handle_picker_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Ctrl-C quits even from picker.
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // If there's a current page, return to it; otherwise quit.
                if self.current_page.is_empty() || self.current_page == "(no page selected)" {
                    self.should_quit = true;
                } else {
                    self.show_picker = false;
                    self.picker_state = None;
                }
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(p) = &mut self.picker_state {
                    p.query.push(c);
                    p.selected = 0;
                    p.list_scroll = 0;
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = &mut self.picker_state {
                    p.query.pop();
                    p.selected = 0;
                    p.list_scroll = 0;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let file_index_len = self.file_index.len();
                if let Some(p) = &mut self.picker_state {
                    let filtered_len = p.filtered(&self.file_index).len();
                    if filtered_len > 0 {
                        p.selected = (p.selected + 1).min(filtered_len - 1);
                        p.clamp(&self.file_index, 20);
                    }
                }
                let _ = file_index_len;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = &mut self.picker_state {
                    p.selected = p.selected.saturating_sub(1);
                    p.clamp(&self.file_index, 20);
                }
            }
            KeyCode::Enter => {
                let selected_title: Option<String> = self.picker_state.as_ref().and_then(|p| {
                    let filtered = p.filtered(&self.file_index);
                    filtered.get(p.selected).map(|s| s.to_string())
                });
                if let Some(title) = selected_title {
                    self.show_picker = false;
                    self.picker_state = None;
                    if title != self.current_page {
                        self.navigate_to(title);
                    }
                }
            }
            _ => {}
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────

    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    fn toggle_focus(&mut self) {
        self.focus_state = match self.focus_state {
            FocusState::ScrollMode => FocusState::FocusMode { focused_index: 0 },
            FocusState::FocusMode { .. } => FocusState::ScrollMode,
        };
    }

    /// Navigate backward using session history (REQ-069).
    fn navigate_back(&mut self) {
        if let Some((page, offset)) = self.nav_history.pop() {
            // Push current state onto forward history before going back.
            self.forward_history
                .push((self.current_page.clone(), self.scroll_offset));
            self.current_page = page.clone();
            let abs_path = self
                .file_index
                .iter()
                .find(|(name, _)| name == &page)
                .map(|(_, rel)| self.vault_root.join(rel));
            self.load_page(abs_path);
            self.scroll_offset = offset;
            self.focus_state = FocusState::ScrollMode;
            if self.debug_render {
                eprintln!(
                    "[zetl view] navigate  from={} to={} history_depth={}",
                    &page,
                    self.current_page,
                    self.nav_history.len(),
                );
            }
        }
    }

    /// Navigate forward through history after going back (REQ-069).
    fn navigate_forward(&mut self) {
        if let Some((page, offset)) = self.forward_history.pop() {
            self.nav_history
                .push((self.current_page.clone(), self.scroll_offset));
            let prev = self.current_page.clone();
            self.current_page = page.clone();
            let abs_path = self
                .file_index
                .iter()
                .find(|(name, _)| name == &page)
                .map(|(_, rel)| self.vault_root.join(rel));
            self.load_page(abs_path);
            self.scroll_offset = offset;
            self.focus_state = FocusState::ScrollMode;
            if self.debug_render {
                eprintln!(
                    "[zetl view] navigate  from={} to={} history_depth={}",
                    prev,
                    self.current_page,
                    self.nav_history.len(),
                );
            }
        }
    }

    /// Navigate to `page_title`, pushing the current page onto the history stack (REQ-069).
    fn navigate_to(&mut self, page_title: String) {
        let abs_path = self
            .file_index
            .iter()
            .find(|(name, _)| name == &page_title)
            .map(|(_, rel)| self.vault_root.join(rel));

        // Truncate oldest history when depth exceeds MAX_HISTORY_DEPTH.
        if self.nav_history.len() >= MAX_HISTORY_DEPTH {
            self.nav_history.remove(0);
        }
        self.nav_history
            .push((self.current_page.clone(), self.scroll_offset));
        // New navigation clears the forward history.
        self.forward_history.clear();

        let from = self.current_page.clone();
        self.current_page = page_title;
        self.load_page(abs_path);
        self.scroll_offset = 0;
        self.focus_state = FocusState::ScrollMode;
        self.pages_visited += 1;

        if self.debug_render {
            eprintln!(
                "[zetl view] navigate  from={} to={} history_depth={}",
                from,
                self.current_page,
                self.nav_history.len(),
            );
        }
    }

    /// Read content from `abs_path` and rebuild `content_lines`, `annotated_lines`, `link_map`.
    fn load_page(&mut self, abs_path: Option<PathBuf>) {
        self.content_lines = abs_path
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default();
        let (annotated, link_map) =
            build_annotated_lines(&self.content_lines, &self.page_set, self.color_mode);
        self.annotated_lines = annotated;
        self.link_map = link_map;
        // Clear excerpt cache for stale entries — keep it simple by not clearing
        // (entries for other pages are still valid; only the current page's own content changed).
    }

    /// Scroll so the currently-focused link's line is visible in the viewport.
    fn scroll_to_focused(&mut self) {
        if let FocusState::FocusMode { focused_index } = self.focus_state {
            if let Some(entry) = self.link_map.get(focused_index) {
                let line = entry.line_number;
                if line < self.scroll_offset {
                    self.scroll_offset = line;
                } else if self.viewport_height > 0
                    && line >= self.scroll_offset + self.viewport_height
                {
                    self.scroll_offset = line + 1 - self.viewport_height;
                }
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    /// Return a snapshot of wikilinks currently visible in the main-pane viewport.
    fn visible_links_snapshot(&self) -> Vec<LinkEntry> {
        self.link_map
            .iter()
            .filter(|e| {
                self.viewport_height > 0
                    && e.line_number >= self.scroll_offset
                    && e.line_number < self.scroll_offset + self.viewport_height
            })
            .cloned()
            .collect()
    }

    /// Load and cache the first `n_lines` lines of `page_title`'s content.
    ///
    /// Returns fewer lines when the page has fewer than `n_lines` lines.
    /// Results are cached so disk reads only happen once per page per session.
    fn load_page_excerpt(&mut self, page_title: &str, n_lines: usize) -> Vec<String> {
        const CACHE_MAX: usize = 60; // cache up to focused-card size

        if let Some(cached) = self.excerpt_cache.get(page_title) {
            return cached.iter().take(n_lines).cloned().collect();
        }

        let result: Vec<String> = self
            .file_index
            .iter()
            .find(|(name, _)| name == page_title)
            .and_then(|(_, rel)| {
                let abs = self.vault_root.join(rel);
                std::fs::read_to_string(abs).ok()
            })
            .map(|s| s.lines().take(CACHE_MAX).map(|l| l.to_string()).collect())
            .unwrap_or_default();

        self.excerpt_cache
            .insert(page_title.to_string(), result.clone());
        result.into_iter().take(n_lines).collect()
    }

    /// Clone `annotated_lines`, applying a highlight to the focused link's glyph span.
    fn annotated_lines_with_focus(&self) -> Vec<Line<'static>> {
        let focused_ordinal = match self.focus_state {
            FocusState::FocusMode { focused_index } => {
                // focused_index is an index into visible links, not link_map directly.
                let visible: Vec<&LinkEntry> = self
                    .link_map
                    .iter()
                    .filter(|e| {
                        self.viewport_height > 0
                            && e.line_number >= self.scroll_offset
                            && e.line_number < self.scroll_offset + self.viewport_height
                    })
                    .collect();
                visible.get(focused_index).map(|e| e.ordinal)
            }
            FocusState::ScrollMode => None,
        };

        let Some(ord) = focused_ordinal else {
            return self.annotated_lines.clone();
        };

        let Some(entry) = self.link_map.iter().find(|e| e.ordinal == ord) else {
            return self.annotated_lines.clone();
        };

        let focused_line = entry.line_number;
        let mut lines = self.annotated_lines.clone();

        if let Some(line) = lines.get_mut(focused_line) {
            let is_no_color = self.color_mode == ColorMode::NoColor;
            let plain = format!("[{ord}]");
            let bang = format!("![{ord}]");
            for span in line.spans.iter_mut() {
                if span.content.as_ref() == plain.as_str() || span.content.as_ref() == bang.as_str()
                {
                    if is_no_color {
                        // Wrap with '>' and '<' markers in no-color mode (ADR-017).
                        let new_content = format!(">{}<", span.content.as_ref());
                        *span = Span::raw(new_content);
                    } else {
                        span.style = span.style.reversed().bold();
                    }
                    break;
                }
            }
        }

        lines
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Return a [`Rect`] centred inside `r`, sized to `percent_x`% wide and
/// `percent_y`% tall.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let margin_v = (100 - percent_y.min(100)) / 2;
    let margin_h = (100 - percent_x.min(100)) / 2;

    let vertical = Layout::vertical([
        Constraint::Percentage(margin_v),
        Constraint::Percentage(percent_y.min(100)),
        Constraint::Percentage(margin_v),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage(margin_h),
        Constraint::Percentage(percent_x.min(100)),
        Constraint::Percentage(margin_h),
    ])
    .split(vertical[1])[1]
}
