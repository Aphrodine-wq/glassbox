//! `glassbox watch` — the live card stream.
//!
//! Tails `~/.glassbox/decisions.jsonl` and re-renders the most recent decisions as
//! full trust-cards as they happen. This is the roadmap's "card stream": the place
//! a human watches an agent being governed in real time. Pure reader (safe beside
//! the live hook), dep-free raw ANSI (no ratatui/crossterm — honors the lean
//! Cargo.toml). Clear-and-redraw rather than the alternate screen, so Ctrl-C
//! leaves the terminal clean without a signal-handler crate.

use crate::card;
use crate::gate::Verdict;
use crate::mode::Mode;
use std::io::Write;

pub fn cmd_watch(args: &[String]) -> i32 {
    let interval_ms = arg_val(args, "--interval")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500u64);
    let max_cards = arg_val(args, "--lines")
        .and_then(|s| s.parse().ok())
        .unwrap_or(6usize);
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{home}/.glassbox/decisions.jsonl");

    // u64::MAX forces a first render; thereafter we redraw only when the file's
    // length changes. A *decrease* (truncation/rotation) just re-reads from the
    // top — no stale offset to corrupt the view.
    let mut last_len: u64 = u64::MAX;
    loop {
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if len != last_len {
            last_len = len;
            redraw(&path, max_cards);
        }
        std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    }
}

fn redraw(path: &str, max_cards: usize) {
    print!("\x1b[2J\x1b[H"); // clear screen + cursor home
    println!("  GLASS BOX — live decision stream  (Ctrl-C to exit)\n");
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let decisions: Vec<serde_json::Value> = text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            if decisions.is_empty() {
                println!("  waiting for decisions…");
            } else {
                let tail = &decisions[decisions.len().saturating_sub(max_cards)..];
                for e in tail {
                    if let Some(c) = render_line(e) {
                        println!("{c}\n");
                    }
                }
            }
        }
        Err(_) => println!("  waiting for decisions…  (no log yet at {path})"),
    }
    let _ = std::io::stdout().flush();
}

/// Reconstruct a trust-card from a stored decision line. Returns None if the line
/// lacks the fields needed to render (e.g. a pre-provenance legacy line).
fn render_line(e: &serde_json::Value) -> Option<String> {
    let action = e.get("action")?.as_str()?;
    let target = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let reason = e.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let blocked = e.get("blocked").and_then(|v| v.as_bool()).unwrap_or(false);
    let mode = Mode::resolve(e.get("mode").and_then(|v| v.as_str()));
    let verdicts: Vec<Verdict> = e
        .get("verdicts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|vj| Verdict {
                    rail: vj
                        .get("rail")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    refused: vj.get("refused").and_then(|x| x.as_bool()).unwrap_or(false),
                    reason: vj
                        .get("reason")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    policy: vj
                        .get("policy")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(card::render(
        action, target, &verdicts, blocked, reason, mode,
    ))
}

fn arg_val<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_line_reconstructs_a_card() {
        let e = serde_json::json!({
            "action": "git push origin main --force",
            "target": "git",
            "reason": "safety rail: contains forbidden substring '--force'",
            "blocked": true,
            "mode": "shadow",
            "verdicts": [
                {"rail": "values", "refused": false, "reason": "clean", "policy": ""},
                {"rail": "safety", "refused": true, "reason": "contains forbidden substring '--force'", "policy": "Irreversible"}
            ]
        });
        let card = render_line(&e).expect("should render");
        assert!(card.contains("[SHADOW]"));
        assert!(card.contains("WOULD-REFUSE"));
        assert!(card.contains("git push origin main --force"));
    }

    #[test]
    fn render_line_skips_lines_without_action() {
        assert!(render_line(&serde_json::json!({"blocked": false})).is_none());
    }
}
