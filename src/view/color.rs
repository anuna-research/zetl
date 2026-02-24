//! Terminal color capability detection and the bridge color palette.
//!
//! Implements NFR-025 and ADR-017: detect color support from environment
//! variables and provide [`LinkColor`] / [`LinkColors`] for anchor coloring.

use ratatui::style::Color;

// ── ColorMode ─────────────────────────────────────────────────────────────

/// Detected terminal color capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// No color: `NO_COLOR` is set (any value) or `TERM=dumb`.
    NoColor,
    /// 8-color ANSI fallback.
    Ansi8,
    /// 256-color: `TERM` contains `256color`.
    Color256,
    /// True-color: `COLORTERM=truecolor` or `COLORTERM=24bit`.
    TrueColor,
}

/// Detect terminal color capability from environment variables.
///
/// Detection priority (NFR-025, ADR-017):
/// 1. `NO_COLOR` set (any value) **or** `TERM=dumb` → [`ColorMode::NoColor`]
/// 2. `COLORTERM=truecolor` **or** `COLORTERM=24bit` → [`ColorMode::TrueColor`]
/// 3. `TERM` contains `256color` → [`ColorMode::Color256`]
/// 4. Fallback → [`ColorMode::Ansi8`]
pub fn detect_color_mode() -> ColorMode {
    // Rule 1: no-color overrides everything.
    if std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").as_deref() == Ok("dumb")
    {
        return ColorMode::NoColor;
    }

    // Rule 2: true-color.
    match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => return ColorMode::TrueColor,
        _ => {}
    }

    // Rule 3: 256-color.
    if std::env::var("TERM").map_or(false, |t| t.contains("256color")) {
        return ColorMode::Color256;
    }

    // Rule 4: 8-color ANSI fallback.
    ColorMode::Ansi8
}

/// Returns `true` when the terminal has no color support.
///
/// Equivalent to `detect_color_mode() == ColorMode::NoColor`.
pub fn no_color() -> bool {
    detect_color_mode() == ColorMode::NoColor
}

// ── Palettes ──────────────────────────────────────────────────────────────

/// Eight visually distinct colors for 8-color ANSI terminals.
///
/// Uses the six main ANSI foreground colors plus two bright variants.
/// On strict 8-color terminals, bright variants degrade to their base color.
const ANSI_PALETTE: [Color; 8] = [
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::LightRed,
    Color::LightGreen,
];

/// Eight visually distinct colors for 256-color terminals.
///
/// Uses `Color::Indexed` entries from the xterm-256 palette chosen for
/// high saturation and mutual distinctness.
const COLOR_256_PALETTE: [Color; 8] = [
    Color::Indexed(196), // bright red
    Color::Indexed(46),  // bright green
    Color::Indexed(226), // yellow
    Color::Indexed(27),  // medium blue
    Color::Indexed(201), // magenta / hot pink
    Color::Indexed(51),  // cyan
    Color::Indexed(208), // orange
    Color::Indexed(165), // purple
];

/// Eight visually distinct colors for true-color terminals.
///
/// RGB values chosen for high saturation and mutual distinctness across
/// both dark and light terminal themes.
const TRUE_COLOR_PALETTE: [Color; 8] = [
    Color::Rgb(231, 76, 60),   // tomato red
    Color::Rgb(39, 174, 96),   // emerald green
    Color::Rgb(241, 196, 15),  // sunflower yellow
    Color::Rgb(52, 152, 219),  // sky blue
    Color::Rgb(155, 89, 182),  // amethyst purple
    Color::Rgb(26, 188, 156),  // turquoise
    Color::Rgb(230, 126, 34),  // carrot orange
    Color::Rgb(236, 112, 178), // rose pink
];

// ── LinkColor ─────────────────────────────────────────────────────────────

/// A single link color wrapping [`ratatui::style::Color`].
///
/// Resolved from a palette of 8 visually distinct colors; the exact values
/// depend on the active [`ColorMode`]. In [`ColorMode::NoColor`] the inner
/// value is [`Color::Reset`] and callers MUST NOT apply it as a foreground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkColor(Color);

impl LinkColor {
    /// Return the [`LinkColor`] for palette index `idx` (0-based, wraps at 8)
    /// under `mode`.
    pub fn from_index(idx: usize, mode: ColorMode) -> Self {
        let i = idx % 8;
        let color = match mode {
            ColorMode::NoColor => Color::Reset,
            ColorMode::Ansi8 => ANSI_PALETTE[i],
            ColorMode::Color256 => COLOR_256_PALETTE[i],
            ColorMode::TrueColor => TRUE_COLOR_PALETTE[i],
        };
        LinkColor(color)
    }

    /// The underlying [`ratatui::style::Color`].
    pub fn color(self) -> Color {
        self.0
    }
}

// ── LinkColors ────────────────────────────────────────────────────────────

/// Assigns [`LinkColor`] values to link ordinals 1..=N, cycling on overflow.
///
/// Ordinals are **1-based**: ordinal 1 maps to palette index 0, ordinal 2 to
/// palette index 1, and so on. When there are more than 8 links the palette
/// cycles from the beginning (ordinal 9 maps to the same color as ordinal 1).
pub struct LinkColors {
    mode: ColorMode,
}

impl LinkColors {
    /// Create a [`LinkColors`] with an explicit [`ColorMode`].
    pub fn new(mode: ColorMode) -> Self {
        Self { mode }
    }

    /// Create a [`LinkColors`] by detecting the current terminal's capability.
    pub fn detect() -> Self {
        Self::new(detect_color_mode())
    }

    /// Return the [`LinkColor`] for link ordinal `n` (1-based).
    ///
    /// The ordinal cycles through the 8-entry palette; ordinal 0 is treated
    /// as ordinal 1 (i.e., `n.saturating_sub(1)` is used as the 0-based index).
    pub fn get(&self, n: usize) -> LinkColor {
        LinkColor::from_index(n.saturating_sub(1), self.mode)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: run a closure with a specific set of env vars, then restore.
    // We use a mutex to avoid races between parallel tests that mutate env.
    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce() -> R, R>(vars: &[(&str, Option<&str>)], f: F) -> R {
        let _guard = ENV_LOCK.lock().unwrap();

        // Save current values.
        let saved: Vec<(&str, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var(k).ok()))
            .collect();

        // Apply desired values.
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }

        let result = f();

        // Restore.
        for (k, v) in &saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }

        result
    }

    #[test]
    fn no_color_env_var_gives_no_color_mode() {
        let mode = with_env(
            &[("NO_COLOR", Some("1")), ("TERM", None), ("COLORTERM", None)],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::NoColor);
    }

    #[test]
    fn term_dumb_gives_no_color_mode() {
        let mode = with_env(
            &[("NO_COLOR", None), ("TERM", Some("dumb")), ("COLORTERM", None)],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::NoColor);
    }

    #[test]
    fn colorterm_truecolor_gives_true_color_mode() {
        let mode = with_env(
            &[
                ("NO_COLOR", None),
                ("TERM", Some("xterm")),
                ("COLORTERM", Some("truecolor")),
            ],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::TrueColor);
    }

    #[test]
    fn colorterm_24bit_gives_true_color_mode() {
        let mode = with_env(
            &[
                ("NO_COLOR", None),
                ("TERM", Some("xterm")),
                ("COLORTERM", Some("24bit")),
            ],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::TrueColor);
    }

    #[test]
    fn term_256color_gives_color_256_mode() {
        let mode = with_env(
            &[
                ("NO_COLOR", None),
                ("TERM", Some("xterm-256color")),
                ("COLORTERM", None),
            ],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::Color256);
    }

    #[test]
    fn fallback_gives_ansi8_mode() {
        let mode = with_env(
            &[
                ("NO_COLOR", None),
                ("TERM", Some("xterm")),
                ("COLORTERM", None),
            ],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::Ansi8);
    }

    #[test]
    fn no_color_takes_priority_over_colorterm() {
        let mode = with_env(
            &[
                ("NO_COLOR", Some("")),
                ("TERM", Some("xterm")),
                ("COLORTERM", Some("truecolor")),
            ],
            detect_color_mode,
        );
        assert_eq!(mode, ColorMode::NoColor);
    }

    #[test]
    fn link_colors_ordinal_1_is_first_palette_entry() {
        let lc = LinkColors::new(ColorMode::Ansi8);
        assert_eq!(lc.get(1).color(), ANSI_PALETTE[0]);
    }

    #[test]
    fn link_colors_ordinal_8_is_last_palette_entry() {
        let lc = LinkColors::new(ColorMode::Ansi8);
        assert_eq!(lc.get(8).color(), ANSI_PALETTE[7]);
    }

    #[test]
    fn link_colors_ordinal_9_cycles_to_first() {
        let lc = LinkColors::new(ColorMode::Ansi8);
        assert_eq!(lc.get(9).color(), lc.get(1).color());
    }

    #[test]
    fn link_colors_no_color_always_reset() {
        let lc = LinkColors::new(ColorMode::NoColor);
        for n in 1..=16 {
            assert_eq!(lc.get(n).color(), Color::Reset);
        }
    }

    #[test]
    fn link_colors_true_color_returns_rgb() {
        let lc = LinkColors::new(ColorMode::TrueColor);
        assert!(matches!(lc.get(1).color(), Color::Rgb(_, _, _)));
    }

    #[test]
    fn link_colors_256_returns_indexed() {
        let lc = LinkColors::new(ColorMode::Color256);
        assert!(matches!(lc.get(1).color(), Color::Indexed(_)));
    }

    #[test]
    fn link_colors_ordinal_zero_treated_as_first() {
        let lc = LinkColors::new(ColorMode::Ansi8);
        // saturating_sub(1) on 0 gives 0, same as ordinal 1.
        assert_eq!(lc.get(0).color(), lc.get(1).color());
    }
}
