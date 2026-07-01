//! Glassbox Fleet — the multi-agent governance dashboard.
//!
//! Three-pane TUI: agents (left), live governance cards (center), cost summary
//! (right). Tails `~/.glassbox/decisions.jsonl` and derives agent registry +
//! token costs from the decision stream — no separate config needed.

pub mod app;
pub mod input;
pub mod ui;

use app::App;
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::time::Duration;

pub fn run(args: &[String]) -> i32 {
    let interval_ms: u64 = arg_val(args, "--interval")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    if let Err(e) = run_tui(interval_ms) {
        eprintln!("fleet: {e}");
        return 1;
    }
    0
}

fn run_tui(interval_ms: u64) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.reload();

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(Duration::from_millis(interval_ms))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match input::handle_key(key.code, &mut app) {
                        input::Action::Quit => break,
                        input::Action::Continue => {}
                    }
                }
            }
        }

        // Reload decisions on each tick (cheap — just stats the file first).
        app.reload();
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn arg_val<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
}
