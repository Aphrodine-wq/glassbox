//! Keyboard input handling for the fleet TUI.

use super::app::App;
use crossterm::event::KeyCode;

pub enum Action {
    Quit,
    Continue,
}

pub fn handle_key(code: KeyCode, app: &mut App) -> Action {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,

        // Pane switching.
        KeyCode::Tab => {
            app.active_pane = (app.active_pane + 1) % 3;
            Action::Continue
        }
        KeyCode::BackTab => {
            app.active_pane = if app.active_pane == 0 { 2 } else { app.active_pane - 1 };
            Action::Continue
        }

        // Navigation within active pane.
        KeyCode::Up | KeyCode::Char('k') => {
            match app.active_pane {
                0 => {
                    // Agent list.
                    if app.selected_agent > 0 {
                        app.selected_agent -= 1;
                        app.decision_scroll = 0;
                    }
                }
                1 => {
                    // Decision scroll.
                    if app.decision_scroll > 0 {
                        app.decision_scroll -= 1;
                    }
                }
                _ => {}
            }
            Action::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            match app.active_pane {
                0 => {
                    // Agent list.
                    if !app.agents.is_empty() && app.selected_agent < app.agents.len() - 1 {
                        app.selected_agent += 1;
                        app.decision_scroll = 0;
                    }
                }
                1 => {
                    // Decision scroll.
                    app.decision_scroll += 1;
                }
                _ => {}
            }
            Action::Continue
        }

        // Toggle show all agents' decisions.
        KeyCode::Char('a') => {
            app.show_all = !app.show_all;
            Action::Continue
        }

        // Mode toggle for selected agent (placeholder — would send command).
        KeyCode::Char('m') => {
            // Future: toggle shadow/enforce for selected agent.
            Action::Continue
        }

        _ => Action::Continue,
    }
}
