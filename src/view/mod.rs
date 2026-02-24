//! `zetl view` — Xanadu-inspired transclusion TUI (SPEC-009, IMPL-009).
//!
//! This module is distinct from [`crate::tui`], which provides the
//! multi-tab dashboard TUI. `view` implements a focused two-pane layout:
//! a main note pane on the left, context cards on the right, connected by
//! a bridge column — a terminal-native take on Ted Nelson's parallel pages.

pub mod app;
pub mod color;
pub mod event;
pub mod terminal;

pub use app::{ContextMode, FocusState, ViewApp};
pub use color::{detect_color_mode, no_color, ColorMode, LinkColor, LinkColors};
pub use terminal::{enter_alternate_screen, restore_terminal};
