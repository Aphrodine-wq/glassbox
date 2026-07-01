//! Fleet TUI — visual design.
//!
//! Layout:
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │  ◆ GLASSBOX FLEET                              ● LIVE   4 agents  $4.50│
//! ├─────────────────┬──────────────────────────────────┬─────────────────────┤
//! │  AGENTS         │  GOVERNANCE STREAM               │  OVERVIEW           │
//! │                 │                                  │                     │
//! │  ▸ claude-code  │  ┌─────────────────────────────┐ │  ┌ Fleet ─────────┐│
//! │    ████░░ ENF   │  │ ✓  git status          14:50│ │  │ $4.50  16 dec  ││
//! │    $2.29  4 dec │  │ ⛔ rm -rf /tmp  BLOCKED 14:50│ │  │ 43.8% blocked  ││
//! │                 │  │ ✓  cargo build         14:50│ │  └────────────────┘│
//! │    g-rump       │  └─────────────────────────────┘ │                     │
//! │    ██░░░░ SHD   │                                  │  ┌ Tokens ────────┐│
//! │    $1.04  3 dec │                                  │  │ ▁▃▅▇█▅▃ 110.8K ││
//! │                 │                                  │  └────────────────┘│
//! ├─────────────────┴──────────────────────────────────┴─────────────────────┤
//! │  Tab pane  j/k nav  a all  q quit                                       │
//! └──────────────────────────────────────────────────────────────────────────┘

use super::app::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

// ── Color palette ──────────────────────────────────────────────────────────
const ACCENT: Color = Color::Rgb(0, 210, 200);    // teal
const ACCENT_DIM: Color = Color::Rgb(0, 120, 115); // muted teal
const BG_CARD: Color = Color::Rgb(30, 30, 42);     // card background
const FG_DIM: Color = Color::Rgb(100, 100, 120);   // muted text
const FG_MID: Color = Color::Rgb(160, 160, 180);   // secondary text
const FG_BRIGHT: Color = Color::Rgb(220, 220, 235); // primary text
const GREEN: Color = Color::Rgb(80, 220, 120);     // allow
const RED: Color = Color::Rgb(255, 85, 85);        // block
const YELLOW: Color = Color::Rgb(255, 200, 60);    // warning / cost
const BLUE: Color = Color::Rgb(100, 140, 255);     // shadow mode
const ORANGE: Color = Color::Rgb(255, 150, 50);    // would-refuse
const SURFACE: Color = Color::Rgb(22, 22, 32);     // main bg

pub fn draw(f: &mut Frame, app: &App) {
    // Full background.
    let bg = Block::default().style(Style::default().bg(SURFACE));
    f.render_widget(bg, f.area());

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(0),    // body
            Constraint::Length(2), // footer
        ])
        .split(f.area());

    draw_header(f, outer[0], app);
    draw_body(f, outer[1], app);
    draw_footer(f, outer[2], app);
}

// ── Header ─────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let is_live = !app.agents.is_empty();

    let mut spans = vec![
        Span::styled(" \u{25c6} ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(
            "GLASSBOX FLEET",
            Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  \u{2502}  ", Style::default().fg(FG_DIM)),
    ];

    if is_live {
        spans.push(Span::styled(
            "\u{25cf} ",
            Style::default().fg(GREEN),
        ));
        spans.push(Span::styled("LIVE", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)));
    } else {
        spans.push(Span::styled(
            "\u{25cb} ",
            Style::default().fg(YELLOW),
        ));
        spans.push(Span::styled("WAITING", Style::default().fg(YELLOW)));
    }

    spans.push(Span::styled("  \u{2502}  ", Style::default().fg(FG_DIM)));
    spans.push(Span::styled(
        format!("{}", app.agents.len()),
        Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" agents  ", Style::default().fg(FG_DIM)));
    spans.push(Span::styled(
        format!("{}", app.total_decisions),
        Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" decisions  ", Style::default().fg(FG_DIM)));

    if app.total_blocked > 0 {
        spans.push(Span::styled(
            format!("{}", app.total_blocked),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" blocked  ", Style::default().fg(FG_DIM)));
    }

    if app.total_cost_usd > 0.0 {
        spans.push(Span::styled(
            format!("${:.2}", app.total_cost_usd),
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" spent", Style::default().fg(FG_DIM)));
    }

    let header = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(SURFACE))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(FG_DIM)),
        );
    f.render_widget(header, area);
}

// ── Body ───────────────────────────────────────────────────────────────────

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(26),       // agents
            Constraint::Percentage(52), // stream
            Constraint::Min(24),       // overview
        ])
        .split(area);

    draw_agents_pane(f, chunks[0], app);
    draw_decisions_pane(f, chunks[1], app);
    draw_overview_pane(f, chunks[2], app);
}

// ── Agents pane ────────────────────────────────────────────────────────────

fn draw_agents_pane(f: &mut Frame, area: Rect, app: &App) {
    let active = app.active_pane == 0;
    let border_color = if active { ACCENT } else { FG_DIM };

    if app.agents.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  waiting for agents\u{2026}", Style::default().fg(FG_DIM))),
            Line::from(""),
            Line::from(Span::styled("  run: glassbox demo", Style::default().fg(ACCENT_DIM))),
        ])
        .block(
            Block::default()
                .title(Span::styled(" Agents ", Style::default().fg(border_color).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(SURFACE)),
        );
        f.render_widget(empty, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();

    for (i, agent) in app.agents.iter().enumerate() {
        let selected = i == app.selected_agent;

        // Status dot color.
        let dot_color = if agent.blocked_count > 0 {
            RED
        } else if agent.would_block_count > 0 {
            ORANGE
        } else {
            GREEN
        };

        // Mode badge.
        let (mode_label, mode_fg, mode_bg) = if agent.mode == "enforce" {
            ("ENF", Color::White, Color::Rgb(180, 40, 40))
        } else {
            ("SHD", Color::White, BLUE)
        };

        // Activity sparkline — mini bar chart of last 20 decisions.
        let spark = render_activity_bar(&agent.activity, area.width.saturating_sub(6) as usize);

        // Name line.
        let name_style = if selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG_BRIGHT)
        };

        let pointer = if selected { "\u{25b8}" } else { " " };
        let pointer_style = if selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG_DIM)
        };

        let line1 = Line::from(vec![
            Span::styled(format!(" {pointer} "), pointer_style),
            Span::styled("\u{25cf} ", Style::default().fg(dot_color)),
            Span::styled(truncate(&agent.name, 14), name_style),
            Span::raw(" "),
            Span::styled(
                format!(" {mode_label} "),
                Style::default().fg(mode_fg).bg(mode_bg),
            ),
        ]);

        // Spark line.
        let line2 = Line::from(vec![
            Span::raw("     "),
            Span::styled(spark, Style::default().fg(if selected { ACCENT_DIM } else { FG_DIM })),
        ]);

        // Stats line.
        let line3 = Line::from(vec![
            Span::raw("     "),
            Span::styled(
                format!("{} dec", agent.total_decisions),
                Style::default().fg(FG_DIM),
            ),
            Span::styled("  ", Style::default()),
            if agent.blocked_count + agent.would_block_count > 0 {
                Span::styled(
                    format!("{} blk", agent.blocked_count + agent.would_block_count),
                    Style::default().fg(RED),
                )
            } else {
                Span::styled("0 blk", Style::default().fg(FG_DIM))
            },
            Span::styled("  ", Style::default()),
            if agent.cost_usd > 0.0 {
                Span::styled(
                    format!("${:.2}", agent.cost_usd),
                    Style::default().fg(YELLOW),
                )
            } else {
                Span::styled("\u{2014}", Style::default().fg(FG_DIM))
            },
        ]);

        // Separator line (dim).
        let line4 = Line::from(Span::styled(
            format!("  {}", "\u{2500}".repeat(area.width.saturating_sub(4) as usize)),
            Style::default().fg(Color::Rgb(40, 40, 55)),
        ));

        items.push(ListItem::new(vec![line1, line2, line3, line4]));
    }

    let list = List::new(items).block(
        Block::default()
            .title(Span::styled(
                " Agents ",
                Style::default().fg(border_color).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(SURFACE)),
    );
    f.render_widget(list, area);
}

// ── Decision stream pane ───────────────────────────────────────────────────

fn draw_decisions_pane(f: &mut Frame, area: Rect, app: &App) {
    let active = app.active_pane == 1;
    let border_color = if active { ACCENT } else { FG_DIM };

    let filtered = app.filtered_decisions();
    let card_height: u16 = 4; // lines per decision card
    let inner_h = area.height.saturating_sub(2);
    let max_visible = (inner_h / card_height) as usize;

    let total = filtered.len();
    let start = if total > max_visible {
        let max_scroll = total - max_visible;
        let scroll = app.decision_scroll.min(max_scroll);
        total - max_visible - scroll
    } else {
        0
    };
    let end = total.min(start + max_visible);
    let visible = &filtered[start..end];

    let mut items: Vec<ListItem> = Vec::new();

    for d in visible {
        let (icon, verdict_text, verdict_color) = match d.decision.as_str() {
            "deny" => ("\u{2716}", "BLOCKED", RED),           // ✖
            "would-refuse" => ("\u{25b2}", "WOULD-REFUSE", ORANGE), // ▲
            "allow" => ("\u{2714}", "ALLOW", GREEN),          // ✔
            "would-allow" => ("\u{2714}", "allow", GREEN),
            _ => ("\u{2022}", &*d.decision, FG_MID),
        };

        let time_str = format_time(d.t);
        let max_action = (area.width as usize).saturating_sub(30);
        let action_str = truncate(&d.action, max_action);

        // Rail badges.
        let rail_spans: Vec<Span> = d
            .rails
            .iter()
            .map(|(rail, refused)| {
                if *refused {
                    Span::styled(
                        format!(" {rail}\u{2716} "),
                        Style::default().fg(RED).bg(Color::Rgb(60, 20, 20)),
                    )
                } else {
                    Span::styled(
                        format!(" {rail}\u{2714} "),
                        Style::default().fg(GREEN).bg(Color::Rgb(20, 50, 30)),
                    )
                }
            })
            .collect();

        // Line 1: icon + verdict + action.
        let line1 = Line::from(vec![
            Span::styled(format!("  {icon} "), Style::default().fg(verdict_color)),
            Span::styled(
                format!("{verdict_text:<14}"),
                Style::default().fg(verdict_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(action_str, Style::default().fg(FG_BRIGHT)),
        ]);

        // Line 2: time + rails + reason.
        let mut line2_spans = vec![
            Span::raw("     "),
            Span::styled(
                format!("{time_str}  "),
                Style::default().fg(FG_DIM),
            ),
        ];
        line2_spans.extend(rail_spans);
        if d.blocked {
            let max_reason = (area.width as usize).saturating_sub(45);
            line2_spans.push(Span::styled(
                format!("  {}", truncate(&d.reason, max_reason)),
                Style::default().fg(Color::Rgb(200, 80, 80)),
            ));
        }
        let line2 = Line::from(line2_spans);

        // Line 3: agent name (when showing all).
        let line3 = if app.show_all {
            Line::from(vec![
                Span::raw("     "),
                Span::styled(
                    format!("\u{2514}\u{2500} {}", d.agent),
                    Style::default().fg(FG_DIM),
                ),
            ])
        } else {
            Line::from(Span::styled(
                format!("  {}", "\u{2500}".repeat(area.width.saturating_sub(4) as usize)),
                Style::default().fg(Color::Rgb(35, 35, 48)),
            ))
        };

        // Separator.
        let line4 = if app.show_all {
            Line::from(Span::styled(
                format!("  {}", "\u{2500}".repeat(area.width.saturating_sub(4) as usize)),
                Style::default().fg(Color::Rgb(35, 35, 48)),
            ))
        } else {
            Line::from("")
        };

        items.push(ListItem::new(vec![line1, line2, line3, line4]));
    }

    let title = if app.show_all {
        " Stream \u{2502} all agents ".to_string()
    } else if !app.agents.is_empty() {
        let name = &app.agents[app.selected_agent].name;
        format!(" Stream \u{2502} {} ", name)
    } else {
        " Stream ".to_string()
    };

    let list = List::new(items).block(
        Block::default()
            .title(Span::styled(
                title,
                Style::default().fg(border_color).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(SURFACE)),
    );
    f.render_widget(list, area);
}

// ── Overview pane (right) ──────────────────────────────────────────────────

fn draw_overview_pane(f: &mut Frame, area: Rect, app: &App) {
    let active = app.active_pane == 2;
    let border_color = if active { ACCENT } else { FG_DIM };

    let mut lines: Vec<Line> = Vec::new();

    // ── Fleet summary card ──
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" \u{25c6} ", Style::default().fg(ACCENT)),
        Span::styled("Fleet", Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(area.width.saturating_sub(3) as usize)),
        Style::default().fg(Color::Rgb(45, 45, 60)),
    )));

    // Big cost number.
    if app.total_cost_usd > 0.0 {
        lines.push(Line::from(vec![
            Span::styled("  $", Style::default().fg(YELLOW)),
            Span::styled(
                format!("{:.2}", app.total_cost_usd),
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  total spend", Style::default().fg(FG_DIM)),
        ]));
    }

    lines.push(Line::from(""));

    // Stats grid.
    let block_rate = if app.total_decisions > 0 {
        (app.total_blocked as f64 / app.total_decisions as f64) * 100.0
    } else {
        0.0
    };

    lines.push(Line::from(vec![
        Span::styled("  agents    ", Style::default().fg(FG_DIM)),
        Span::styled(
            format!("{}", app.agents.len()),
            Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  decisions ", Style::default().fg(FG_DIM)),
        Span::styled(
            format!("{}", app.total_decisions),
            Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  blocked   ", Style::default().fg(FG_DIM)),
        Span::styled(
            format!("{}", app.total_blocked),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  rate      ", Style::default().fg(FG_DIM)),
        Span::styled(
            format!("{:.1}%", block_rate),
            Style::default()
                .fg(if block_rate > 20.0 {
                    RED
                } else if block_rate > 5.0 {
                    ORANGE
                } else {
                    GREEN
                })
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Block rate visual bar.
    let bar_w = area.width.saturating_sub(5) as usize;
    let filled = ((block_rate / 100.0) * bar_w as f64) as usize;
    let bar_color = if block_rate > 20.0 { RED } else if block_rate > 5.0 { ORANGE } else { GREEN };
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "\u{2588}".repeat(filled),
            Style::default().fg(bar_color),
        ),
        Span::styled(
            "\u{2591}".repeat(bar_w.saturating_sub(filled)),
            Style::default().fg(Color::Rgb(40, 40, 55)),
        ),
    ]));

    // ── Per-agent costs ──
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" \u{25c6} ", Style::default().fg(ACCENT)),
        Span::styled("Cost Breakdown", Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(area.width.saturating_sub(3) as usize)),
        Style::default().fg(Color::Rgb(45, 45, 60)),
    )));

    let max_cost = app.agents.iter().map(|a| a.cost_usd).fold(0.0f64, f64::max);

    for agent in &app.agents {
        let total_tokens = agent.tokens_in + agent.tokens_out;

        // Cost bar relative to highest spender.
        let cost_bar_w = area.width.saturating_sub(6) as usize;
        let cost_fill = if max_cost > 0.0 {
            ((agent.cost_usd / max_cost) * cost_bar_w as f64) as usize
        } else {
            0
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", truncate(&agent.name, 12)),
                Style::default().fg(FG_MID),
            ),
        ]));

        if agent.cost_usd > 0.0 {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "\u{2583}".repeat(cost_fill),
                    Style::default().fg(YELLOW),
                ),
                Span::styled(
                    "\u{2581}".repeat(cost_bar_w.saturating_sub(cost_fill)),
                    Style::default().fg(Color::Rgb(40, 40, 55)),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  ${:.2}", agent.cost_usd),
                    Style::default().fg(YELLOW),
                ),
                Span::styled(
                    format!("  {}tok", format_tokens(total_tokens)),
                    Style::default().fg(FG_DIM),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  no cost data", Style::default().fg(FG_DIM)),
            ]));
        }
        lines.push(Line::from(""));
    }

    // ── Token breakdown ──
    let total_in: u64 = app.agents.iter().map(|a| a.tokens_in).sum();
    let total_out: u64 = app.agents.iter().map(|a| a.tokens_out).sum();
    if total_in + total_out > 0 {
        lines.push(Line::from(vec![
            Span::styled(" \u{25c6} ", Style::default().fg(ACCENT)),
            Span::styled("Tokens", Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(Span::styled(
            format!(" {}", "\u{2500}".repeat(area.width.saturating_sub(3) as usize)),
            Style::default().fg(Color::Rgb(45, 45, 60)),
        )));
        lines.push(Line::from(vec![
            Span::styled("  \u{25b8} in   ", Style::default().fg(FG_DIM)),
            Span::styled(format_tokens(total_in), Style::default().fg(ACCENT)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  \u{25b8} out  ", Style::default().fg(FG_DIM)),
            Span::styled(format_tokens(total_out), Style::default().fg(ACCENT)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  \u{25b8} total ", Style::default().fg(FG_DIM)),
            Span::styled(
                format_tokens(total_in + total_out),
                Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    let panel = Paragraph::new(lines)
        .block(
            Block::default()
                .title(Span::styled(
                    " Overview ",
                    Style::default().fg(border_color).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(SURFACE)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(panel, area);
}

// ── Footer ─────────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let pane_names = ["Agents", "Stream", "Overview"];
    let mut spans = vec![Span::raw(" ")];

    for (i, name) in pane_names.iter().enumerate() {
        if i == app.active_pane as usize {
            spans.push(Span::styled(
                format!(" {name} "),
                Style::default()
                    .fg(Color::Rgb(15, 15, 25))
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {name} "),
                Style::default().fg(FG_DIM),
            ));
        }
        spans.push(Span::raw(" "));
    }

    spans.push(Span::styled(" \u{2502} ", Style::default().fg(FG_DIM)));

    // Keybind hints.
    let keys = [
        ("Tab", "pane"),
        ("j/k", "nav"),
        ("a", if app.show_all { "filter" } else { "all" }),
        ("q", "quit"),
    ];
    for (key, desc) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default().fg(SURFACE).bg(FG_DIM),
        ));
        spans.push(Span::styled(
            format!(" {desc} "),
            Style::default().fg(FG_DIM),
        ));
    }

    let footer = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(SURFACE))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(FG_DIM)),
        );
    f.render_widget(footer, area);
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Render a mini activity bar from recent decisions.
/// Green block = allowed, red block = blocked, dim = empty slot.
fn render_activity_bar(activity: &[bool], max_width: usize) -> String {
    let width = max_width.min(20);
    let mut bar = String::new();
    for i in 0..width {
        if i < activity.len() {
            if activity[i] {
                bar.push('\u{2588}'); // blocked = full block (will be colored red by context)
            } else {
                bar.push('\u{2584}'); // allowed = lower half block
            }
        } else {
            bar.push('\u{2581}'); // empty slot
        }
    }
    bar
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}

fn format_time(t: u64) -> String {
    if t == 0 {
        return "\u{2014}".to_string();
    }
    let secs = (t / 1000) as i64;
    match chrono::DateTime::from_timestamp(secs, 0) {
        Some(d) => d.format("%H:%M:%S").to_string(),
        None => "\u{2014}".to_string(),
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}
