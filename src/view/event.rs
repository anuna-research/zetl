use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::prelude::*;

use super::app::ViewApp;

/// Poll-and-dispatch loop: draw → wait for event → handle → repeat.
///
/// Exits cleanly when [`ViewApp::should_quit`] is set to `true`.
pub fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ViewApp,
) -> Result<()> {
    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code, key.modifiers);
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
