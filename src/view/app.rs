use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Clear, Paragraph};

use super::terminal::{enter_alternate_screen, restore_terminal};
use crate::view::event::run_event_loop;

/// Terminal width below which the view switches to single-pane mode (REQ-063).
const SINGLE_PANE_THRESHOLD: u16 = 60;

/// Width of the bridge column separating the two panes (NFR-025).
const BRIDGE_WIDTH: u16 = 3;

// ── ContextMode ───────────────────────────────────────────────────────────

/// Which set of context cards to display in the right pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextMode {
    /// Show forward links from the current page (default).
    #[default]
    Forward,
    /// Show backlinks — pages that link to the current page.
    Back,
    /// Show both forward links and backlinks separated by a divider.
    Both,
}

impl ContextMode {
    pub fn cycle(self) -> Self {
        match self {
            ContextMode::Forward => ContextMode::Back,
            ContextMode::Back => ContextMode::Both,
            ContextMode::Both => ContextMode::Forward,
        }
    }
}

// ── FocusState ────────────────────────────────────────────────────────────

/// Whether the user is scrolling freely or has focused a specific wikilink.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

    /// Number of lines scrolled down from the top of the document.
    pub scroll_offset: usize,

    /// Height (in terminal rows) of the scrollable content area — refreshed every draw call.
    /// Used by Ctrl-d/Ctrl-u for half-page scrolling and by G for go-to-bottom (REQ-067).
    pub viewport_height: usize,

    /// Navigation history as `(page_title, scroll_offset)` pairs.
    /// The most recent entry is pushed when navigating away; popped on `[`.
    pub nav_history: Vec<(String, usize)>,

    /// Which context cards to show in the right pane.
    pub context_mode: ContextMode,

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

    /// Set to `true` to break out of the event loop.
    pub should_quit: bool,

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
    ///
    /// `file_path` is the absolute path to the markdown file for `page`.  When
    /// `Some`, the file is read immediately so the main pane can display its
    /// content (REQ-067).  Errors are silently ignored — `content_lines` will
    /// be empty if the file cannot be read.
    pub fn new(
        page: impl Into<String>,
        file_path: Option<PathBuf>,
        context_lines: u8,
        main_width: u8,
    ) -> Self {
        let current_page = page.into();
        let content_lines: Vec<String> = file_path
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default();

        Self {
            current_page,
            content_lines,
            scroll_offset: 0,
            viewport_height: 0,
            nav_history: Vec::new(),
            context_mode: ContextMode::default(),
            focus_state: FocusState::default(),
            context_lines,
            main_width,
            show_help: false,
            context_overlay: false,
            should_quit: false,
            main_pane: Rect::default(),
            bridge_col: Rect::default(),
            context_pane: Rect::default(),
            status_bar: Rect::default(),
            single_pane: false,
        }
    }

    /// Open the alternate screen, run the event loop, then restore the terminal.
    pub fn run(&mut self) -> Result<()> {
        let mut terminal = enter_alternate_screen()?;
        let result = run_event_loop(&mut terminal, self);
        restore_terminal()?;
        result
    }

    /// Render a single frame into `frame`, updating stored pane rects (REQ-063, NFR-025).
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();

        // ── Step 1: vertical split — content + 1-row status bar ──────────
        let [content_area, status_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        self.status_bar = status_area;

        // ── Step 2: horizontal split based on terminal width ─────────────
        if content_area.width < SINGLE_PANE_THRESHOLD {
            // Single-pane mode: main pane fills the whole content area.
            self.single_pane = true;
            self.main_pane = content_area;
            self.bridge_col = Rect::default();
            self.context_pane = Rect::default();
        } else {
            // Two-pane mode: main | bridge | context.
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

        // ── Step 3: render panes ─────────────────────────────────────────

        self.draw_main_pane(frame);

        if !self.single_pane {
            self.draw_bridge_col(frame);
            self.draw_context_pane(frame);
        }

        // ── Step 4: status bar and overlays ──────────────────────────────
        let status_area = self.status_bar;
        self.draw_status_bar(frame, status_area);

        // Overlays are drawn last so they appear on top.
        if self.context_overlay {
            self.draw_context_overlay(frame, area);
        }
        if self.show_help {
            self.draw_help_overlay(frame, area);
        }
    }

    /// Render the main note pane: a styled 1-row title header followed by the
    /// scrollable Markdown content (REQ-063, REQ-067).
    ///
    /// Word-wrap is intentionally disabled so that raw Markdown lines are
    /// preserved exactly as written in the source file.  The scroll position is
    /// clamped to the valid range on every draw call.
    fn draw_main_pane(&mut self, frame: &mut Frame) {
        let area = self.main_pane;

        // Split into a 1-row header and the remaining scrollable content area.
        let [header_area, content_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

        // Update viewport_height so that half-page and go-to-bottom keybindings
        // use the current actual height (REQ-067).
        self.viewport_height = content_area.height as usize;

        // Clamp scroll_offset so it never exceeds the last displayable position.
        let max_scroll = self.content_lines.len().saturating_sub(self.viewport_height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        // ── Title header ──────────────────────────────────────────────────
        let header = Paragraph::new(format!(" {} ", self.current_page))
            .style(Style::default().bold().reversed());
        frame.render_widget(header, header_area);

        // ── Scrollable content ────────────────────────────────────────────
        // Build Line items from raw content strings.  No word-wrap is applied;
        // long Markdown lines extend past the right edge (horizontal scrolling
        // is not implemented in this iteration).
        let lines: Vec<Line> = self
            .content_lines
            .iter()
            .map(|l| Line::raw(l.as_str()))
            .collect();

        let scroll_row = self.scroll_offset.min(u16::MAX as usize) as u16;
        let para = Paragraph::new(lines).scroll((scroll_row, 0));
        frame.render_widget(para, content_area);
    }

    /// Render the 3-column bridge strip separating the two panes.
    fn draw_bridge_col(&self, frame: &mut Frame) {
        let area = self.bridge_col;
        // Fill each row with " │ " — a centred vertical bar.
        let lines: Vec<Line> = (0..area.height).map(|_| Line::from(" │ ")).collect();
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Render the context pane (placeholder until context-card rendering is implemented).
    fn draw_context_pane(&self, frame: &mut Frame) {
        let mode_str = match self.context_mode {
            ContextMode::Forward => "Forward links",
            ContextMode::Back => "Backlinks",
            ContextMode::Both => "Forward links & Backlinks",
        };
        let block = Block::bordered().title(format!(" {} ", mode_str));
        frame.render_widget(block, self.context_pane);
    }

    /// Render the single-line status bar at `area`.
    fn draw_status_bar(&self, frame: &mut Frame, area: Rect) {
        let mode_str = match self.context_mode {
            ContextMode::Forward => "forward",
            ContextMode::Back => "back",
            ContextMode::Both => "both",
        };

        let focused_segment = match &self.focus_state {
            FocusState::ScrollMode => String::new(),
            FocusState::FocusMode { focused_index } => {
                format!(" │ [{}] {}", focused_index + 1, self.current_page)
            }
        };

        let status = format!(
            " zetl view │ {} │ {} │{} 0 links │ j/k scroll  Tab focus  ?",
            self.current_page, mode_str, focused_segment,
        );

        let para = Paragraph::new(status).style(Style::default().reversed());
        frame.render_widget(para, area);
    }

    /// Render the context-pane full-screen overlay (Ctrl-R, single-pane fallback).
    fn draw_context_overlay(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(80, 80, area);
        let mode_str = match self.context_mode {
            ContextMode::Forward => "Forward links",
            ContextMode::Back => "Backlinks",
            ContextMode::Both => "Forward links & Backlinks",
        };
        let block = Block::bordered().title(format!(" Context pane — {} ", mode_str));
        let inner = block.inner(popup);
        frame.render_widget(Clear, popup);
        frame.render_widget(block, popup);
        let body = Paragraph::new("(no links loaded)").centered();
        frame.render_widget(body, inner);
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

    /// Handle a key event, mutating state.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
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

        // Ctrl-d / Ctrl-u half-page scroll (REQ-067).
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
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_up();
            }
            KeyCode::Char('g') => {
                self.scroll_offset = 0;
            }
            KeyCode::Char('G') => {
                // Go to the last line.  draw_main_pane() will clamp to the
                // exact displayable maximum, but we can compute it precisely
                // when viewport_height is already known (REQ-067).
                self.scroll_offset =
                    self.content_lines.len().saturating_sub(self.viewport_height);
            }
            KeyCode::Tab => {
                self.toggle_focus();
            }
            KeyCode::Enter => {
                // Navigate to the focused link's target page.
                if let FocusState::FocusMode { .. } = self.focus_state {
                    // Placeholder: no link data yet.
                }
            }
            KeyCode::Char('b') => {
                self.context_mode = self.context_mode.cycle();
            }
            KeyCode::Char('[') => {
                self.navigate_back();
            }
            KeyCode::Char(']') => {
                // Forward navigation: placeholder (history not tracked yet).
            }
            _ => {}
        }
    }

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

    fn navigate_back(&mut self) {
        if let Some((page, offset)) = self.nav_history.pop() {
            self.current_page = page;
            self.scroll_offset = offset;
            self.focus_state = FocusState::ScrollMode;
        }
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
