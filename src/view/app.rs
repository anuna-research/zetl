use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Clear, Paragraph};

use super::terminal::{enter_alternate_screen, restore_terminal};
use crate::view::event::run_event_loop;

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

    /// Number of lines scrolled down from the top of the document.
    pub scroll_offset: usize,

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
}

impl ViewApp {
    /// Create a new [`ViewApp`] starting on `page`.
    pub fn new(page: impl Into<String>, context_lines: u8, main_width: u8) -> Self {
        Self {
            current_page: page.into(),
            scroll_offset: 0,
            nav_history: Vec::new(),
            context_mode: ContextMode::default(),
            focus_state: FocusState::default(),
            context_lines,
            main_width,
            show_help: false,
            context_overlay: false,
            should_quit: false,
        }
    }

    /// Open the alternate screen, run the event loop, then restore the terminal.
    pub fn run(&mut self) -> Result<()> {
        let mut terminal = enter_alternate_screen()?;
        let result = run_event_loop(&mut terminal, self);
        restore_terminal()?;
        result
    }

    /// Render a single frame into `frame`.
    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        // Reserve the bottom row for the status bar.
        let [main_area, status_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

        // Main content placeholder.
        let title = format!(" zetl view — {} ", self.current_page);
        let block = Block::bordered().title(title);
        frame.render_widget(block, main_area);

        // Status bar.
        self.draw_status_bar(frame, status_area);

        // Overlays are drawn last so they appear on top.
        if self.context_overlay {
            self.draw_context_overlay(frame, area);
        }
        if self.show_help {
            self.draw_help_overlay(frame, area);
        }
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

        // Ctrl-d / Ctrl-u half-page scroll.
        if modifiers.contains(KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('d') => {
                    self.scroll_offset = self.scroll_offset.saturating_add(10);
                    return;
                }
                KeyCode::Char('u') => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(10);
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
                // Jump to a large offset; the renderer will clamp to content length.
                self.scroll_offset = usize::MAX / 2;
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
