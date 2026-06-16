//! The Glass Box — a governance trust-layer for agents.
//!
//! Makes an agent's actions legible and governed at the moment they happen: each
//! proposed action runs through two readable rails (values + safety) and renders
//! as a plain-language trust-card — refused if a rail says no. First integration:
//! Claude Code governing itself via a PreToolUse hook.

mod audit;
mod card;
mod gate;
mod safety;
mod values;

use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let code = match cmd {
        "hook" => cmd_hook(),
        "demo" => cmd_demo(),
        "gate" => cmd_gate(&args),
        "status" => cmd_status(),
        _ => {
            eprintln!("The Glass Box — governance trust-layer");
            eprintln!("usage: glassbox [hook | demo | gate <action> | status]");
            2
        }
    };
    std::process::exit(code);
}

/// PreToolUse adapter: stdin JSON in, hook protocol out. Blocks on refusal.
fn cmd_hook() -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        println!("{{}}");
        return 0;
    }
    let event: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => {
            println!("{{}}");
            return 0;
        }
    };

    let tool_name = event
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let empty = serde_json::json!({});
    let tool_input = event.get("tool_input").unwrap_or(&empty);

    let (action, target) = gate::perceive(tool_name, tool_input);
    let verdicts = gate::evaluate(&action, &target, false);
    let (blocked, reason) = gate::decide(&verdicts);
    audit::record(&action, &target, &verdicts, blocked, &reason);

    if blocked {
        let out = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format!("Glass Box governance — {}", reason),
            }
        });
        println!("{}", out);
    } else {
        println!("{{}}"); // defer to normal permission flow
    }
    0
}

/// The proof: six proposed actions a real agent might take, each governed.
fn cmd_demo() -> i32 {
    let cases: &[(&str, &str, &str)] = &[
        (
            "Read a config file",
            "cat ~/Projects/walt/README.md",
            "shell",
        ),
        ("Commit work", "git commit -m 'add glass box'", "shell"),
        (
            "Force-push to main (IRREVERSIBLE)",
            "git push origin main --force",
            "shell",
        ),
        (
            "Wipe a directory (IRREVERSIBLE)",
            "rm -rf ~/Projects/walt/glassbox",
            "shell",
        ),
        (
            "Drop a production table (IRREVERSIBLE)",
            "psql -c 'DROP TABLE users'",
            "shell",
        ),
        (
            "Reprice a loyal client (VALUES)",
            "reprice loyal client Casey to market rate",
            "Casey",
        ),
    ];
    println!(
        "\n  The Glass Box — governing six proposed actions a real agent might take.\n  \
         Each is perceived, run through the values + safety rails, and allowed or blocked.\n"
    );
    let mut blocked_count = 0;
    for (label, action, target) in cases {
        let verdicts = gate::evaluate(action, target, true);
        let (blocked, reason) = gate::decide(&verdicts);
        audit::record(action, target, &verdicts, blocked, &reason);
        println!("  ▸ {}", label);
        println!(
            "{}",
            card::render(action, target, &verdicts, blocked, &reason)
        );
        println!();
        blocked_count += blocked as i32;
    }
    println!(
        "  {}/{} actions blocked at the moment of action — each refusal readable, \
         governed by a markdown file you can edit.\n",
        blocked_count,
        cases.len()
    );
    0
}

fn cmd_gate(args: &[String]) -> i32 {
    let action = match args.get(2) {
        Some(a) => a.clone(),
        None => {
            eprintln!("usage: glassbox gate \"<action>\" [target]");
            return 2;
        }
    };
    let target = args.get(3).cloned().unwrap_or_else(|| "shell".to_string());
    let verdicts = gate::evaluate(&action, &target, true);
    let (blocked, reason) = gate::decide(&verdicts);
    audit::record(&action, &target, &verdicts, blocked, &reason);
    println!(
        "{}",
        card::render(&action, &target, &verdicts, blocked, &reason)
    );
    if blocked {
        1
    } else {
        0
    }
}

fn cmd_status() -> i32 {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{}/.glassbox/decisions.jsonl", home);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            println!("  no decisions yet");
            return 0;
        }
    };
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(10)..];
    println!("  last {} decisions:", tail.len());
    for line in tail {
        if let Ok(e) = serde_json::from_str::<serde_json::Value>(line) {
            let mark = if e.get("blocked").and_then(|v| v.as_bool()).unwrap_or(false) {
                "⛔"
            } else {
                "✓"
            };
            let action = e.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let reason = e.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            println!(
                "    {} {:<50} {}",
                mark,
                action.chars().take(50).collect::<String>(),
                reason
            );
        }
    }
    0
}
