//! The safety rail — refuses irreversible operations.
//!
//! Runs fully in-process (no subprocess, no failure mode) so it is always on and
//! sub-millisecond — the part of the gate that must never be down. The forbidden
//! substrings are READ FROM the readable governance file (`agents/safety.t.md`),
//! so the markdown stays the source of truth even on the hot path.

use crate::gate::Verdict;
use std::fs;

fn patterns() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{}/Projects/walt/glassbox/agents/safety.t.md", home);
    let mut pats = Vec::new();
    if let Ok(text) = fs::read_to_string(&path) {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("forbid contains ") {
                let s = rest.trim().trim_matches('"');
                if !s.is_empty() {
                    pats.push(s.to_string());
                }
            }
        }
    }
    // Hard fallback: never let the safety rail run empty, even if the file moves.
    if pats.is_empty() {
        pats = [
            "rm -rf",
            "rm -fr",
            "--force",
            "push -f",
            "reset --hard",
            "drop table",
            "DROP TABLE",
            "truncate",
            "delete from",
            "DELETE FROM",
            "mkfs",
            "dd if=",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
    }
    pats
}

pub fn check(action: &str) -> Verdict {
    for p in patterns() {
        if action.contains(&p) {
            return Verdict {
                rail: "safety".into(),
                refused: true,
                reason: format!("contains forbidden substring '{}'", p),
                policy: "Irreversible".into(),
            };
        }
    }
    Verdict {
        rail: "safety".into(),
        refused: false,
        reason: "clean".into(),
        policy: String::new(),
    }
}
