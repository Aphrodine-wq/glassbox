//! The Glass Box — a governance trust-layer for agents.
//!
//! Makes an agent's actions legible and governed at the moment they happen: each
//! proposed action runs through two readable rails (values + safety) and renders
//! as a plain-language trust-card — refused if a rail says no. First integration:
//! Claude Code governing itself via a PreToolUse hook.

mod audit;
mod card;
mod eval;
mod gate;
mod mode;
mod protocol;
mod provenance;
mod safety;
mod values;
mod watch;

use std::io::Read;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;
    /// Serializes tests that mutate process-global env (HOME, GLASSBOX_MODE).
    /// Run the suite single-threaded for full determinism:
    /// `cargo test -- --test-threads=1`.
    pub static ENV_LOCK: Mutex<()> = Mutex::new(());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let code = match cmd {
        "hook" => cmd_hook(),
        "gate-json" => cmd_gate_json(),
        "demo" => cmd_demo(),
        "gate" => cmd_gate(&args),
        "status" => cmd_status(),
        "watch" => watch::cmd_watch(&args),
        "eval" => eval::cmd_eval(&args),
        _ => {
            eprintln!("The Glass Box — governance trust-layer");
            eprintln!(
                "usage: glassbox [hook | gate-json | demo | gate <action> [target] | status | watch | eval [--values]]"
            );
            2
        }
    };
    std::process::exit(code);
}

/// PreToolUse adapter: stdin JSON in, hook protocol out.
///
/// The body is wrapped in `catch_unwind` — a governance hook must never crash a
/// tool call, so any panic degrades to a silent defer (`{}`). The decision→output
/// mapping goes through [`mode::resolve_output`], so in the default `Shadow` mode
/// the hook is *structurally* incapable of emitting a deny.
fn cmd_hook() -> i32 {
    let out = std::panic::catch_unwind(hook_decide).unwrap_or_else(|_| "{}".to_string());
    println!("{}", out);
    0
}

/// The pure-ish core of the hook: read stdin → perceive (Claude Code adapter) →
/// run the gate → resolve the hook output for the current mode. Returns the
/// stdout JSON. Mode is env-resolved and defaults to Shadow, so the live default
/// can never deny (see [`mode::resolve_output`]).
fn hook_decide() -> String {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return "{}".to_string();
    }
    let event: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => return "{}".to_string(),
    };

    let tool_name = event
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let empty = serde_json::json!({});
    let tool_input = event.get("tool_input").unwrap_or(&empty);

    // perceive() is the Claude Code adapter into the generic gate request.
    let (action, target) = gate::perceive(tool_name, tool_input);
    let req = protocol::GateRequest {
        action,
        target,
        agent: "claude-code".into(),
        mode: mode::Mode::from_env(),
        link: false, // hot path: never the extra Tessera read
    };
    let resp = protocol::run_gate(&req);
    match mode::resolve_output(req.mode, resp.blocked, &resp.reason) {
        deny @ mode::HookOutput::Deny(_) => deny.render(),
        // Defer: in shadow, attach the live one-liner via `systemMessage` so the
        // human SEES the governed decision at the moment of action (the wedge),
        // while the action still proceeds. Enforce-allow stays quiet.
        mode::HookOutput::Defer => {
            if req.mode == mode::Mode::Shadow {
                serde_json::json!({ "systemMessage": resp.summary_line() }).to_string()
            } else {
                "{}".to_string()
            }
        }
    }
}

/// Generic, agent-agnostic adapter: stdin `{action,target,agent,mode}` in, the
/// full `GateResponse` (decision, verdicts, card, provenance_id) out. The real
/// public API — the Claude Code `hook` is just one frontend on top of `run_gate`.
fn cmd_gate_json() -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        println!("{{}}");
        return 1;
    }
    let v: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("glassbox: invalid JSON request");
            println!("{{}}");
            return 1;
        }
    };
    let req = protocol::GateRequest::from_json(&v);
    let resp = protocol::run_gate(&req);
    println!("{}", resp.to_json_with_card());
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
            "reprice loyal client to market rate",
            "loyal-client",
        ),
    ];
    println!(
        "\n  The Glass Box — governing six proposed actions a real agent might take.\n  \
         Each is perceived, run through the values + safety rails, and allowed or blocked.\n"
    );
    let mut blocked_count = 0;
    for (label, action, target) in cases {
        let req = protocol::GateRequest {
            action: action.to_string(),
            target: target.to_string(),
            agent: "demo".into(),
            mode: mode::Mode::Enforce,
            link: true,
        };
        let resp = protocol::run_gate(&req);
        println!("  ▸ {}", label);
        println!("{}", resp.card);
        println!();
        blocked_count += resp.blocked as i32;
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
    let req = protocol::GateRequest {
        action,
        target,
        agent: "cli".into(),
        mode: mode::Mode::Enforce,
        link: true,
    };
    let resp = protocol::run_gate(&req);
    println!("{}", resp.card);
    if resp.blocked {
        1
    } else {
        0
    }
}

fn cmd_status() -> i32 {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{home}/.glassbox/decisions.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            println!("  no decisions yet");
            return 0;
        }
    };
    let parsed: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let blocked_total = parsed
        .iter()
        .filter(|e| e.get("blocked").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    println!(
        "  mode: {} · {} decisions · {} would-block",
        mode::Mode::from_env().label(),
        parsed.len(),
        blocked_total
    );

    let tail = &parsed[parsed.len().saturating_sub(10)..];
    let mut saw_values_refusal = false;
    for e in tail {
        let blocked = e.get("blocked").and_then(|v| v.as_bool()).unwrap_or(false);
        let mark = if blocked { "⛔" } else { "✓" };
        let action = e.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let reason = e.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        println!(
            "    {} {:<50} {}",
            mark,
            action.chars().take(50).collect::<String>(),
            reason
        );
        if let Some(p) = e.get("provenance").filter(|p| !p.is_null()) {
            let val = p.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let esc = p.get("escalation").and_then(|v| v.as_str()).unwrap_or("");
            let src = p.get("source").and_then(|v| v.as_str()).unwrap_or("");
            println!("        ↳ value: {val} · escalate: {esc}");
            if src.contains("conscience") {
                saw_values_refusal = true;
            }
        }
    }

    // Cross-store proof (off the hot path): link the values rail to Tessera's
    // permanent governance graph, showing the two stores agree.
    if saw_values_refusal {
        let mut probe = provenance::Provenance {
            source: "tessera/conscience".into(),
            policy: String::new(),
            value: String::new(),
            intent: String::new(),
            escalation: String::new(),
            tessera_seq: None,
            tessera_created_at: None,
        };
        provenance::link_tessera(&mut probe);
        if let Some(seq) = probe.tessera_seq {
            let at = probe.tessera_created_at.unwrap_or_default();
            println!("  tessera governance: latest Conscience refusal #{seq} @ {at}");
        }
    }
    0
}
