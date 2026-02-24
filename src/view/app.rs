use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph};

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

    /// Set to `true` to break out of the event loop.
    pub should_quit: bool,
}

impl ViewApp {
    /// Create a new [`ViewApp`] starting on `page`.
    pub fn new(page: impl Into<String>) -> Self {
        Self {
            current_page: page.into(),
            scroll_offset: 0,
            nav_history: Vec::new(),
            context_mode: ContextMode::default(),
            focus_state: FocusState::default(),
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
        let title = format!(" zetl view — {} ", self.current_page);
        let block = Block::bordered().title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mode = match self.context_mode {
            ContextMode::Forward => "forward",
            ContextMode::Back => "back",
            ContextMode::Both => "both",
        };
        let focus = match &self.focus_state {
            FocusState::ScrollMode => "scroll".to_string(),
            FocusState::FocusMode { focused_index } => format!("focus[{}]", focused_index),
        };
        let status = format!(
            " scroll:{} mode:{} {}  q quit  Tab focus  b toggle context",
            self.scroll_offset, mode, focus
        );
        let para = Paragraph::new(status);
        frame.render_widget(para, inner);
    }

    /// Handle a key event, mutating state.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Ctrl-C always quits.
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_up();
            }
            KeyCode::Tab => {
                self.toggle_focus();
            }
            KeyCode::Char('b') => {
                self.context_mode = self.context_mode.cycle();
            }
            KeyCode::Char('[') => {
                self.navigate_back();
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
