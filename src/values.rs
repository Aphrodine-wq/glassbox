//! The values rail — refuses what is *wrong* (extractive, unfair, repricing a
//! loyal client). Governed by the Conscience markdown in the mind layer.
//!
//! Two design choices that make it safe to run on the live agent:
//!   1. A cheap in-process keyword pre-screen — the Tessera subprocess only runs
//!      when the action plausibly touches values, so the 99% of commands with
//!      nothing to do with money pay nothing.
//!   2. FAIL-OPEN on infra error/timeout — a rare Tessera hiccup logs and allows
//!      rather than bricking the agent. (Safety, which never fails, stays the
//!      hard floor.)

use crate::gate::Verdict;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const VALUE_KEYWORDS: &[&str] = &[
    "reprice",
    "gouge",
    "squeeze",
    "upcharge",
    "upcharg",
    "charge",
    "invoice",
    "deposit",
    "refund",
    "payment",
    "price",
    "pricing",
    "discount",
    "markup",
    "casey",
    "loyal client",
    "extract",
];

pub fn check(action: &str, target: &str) -> Verdict {
    let hay = format!("{} {}", action, target).to_lowercase();
    let relevant = VALUE_KEYWORDS.iter().any(|k| hay.contains(k));
    if !relevant {
        return clean();
    }
    match run_conscience(action, target) {
        Some((reason, policy)) => Verdict {
            rail: "values".into(),
            refused: true,
            reason,
            policy,
        },
        None => clean(), // clean verdict OR fail-open on infra error
    }
}

fn clean() -> Verdict {
    Verdict {
        rail: "values".into(),
        refused: false,
        reason: "clean".into(),
        policy: String::new(),
    }
}

/// Returns Some((reason, policy)) if Conscience refused, None if it passed OR
/// the gate could not run (fail-open).
fn run_conscience(action: &str, target: &str) -> Option<(String, String)> {
    let home = std::env::var("HOME").ok()?;
    let bin = format!("{}/Projects/walt/tessera/.venv/bin/tessera", home);
    let agent = format!("{}/Projects/walt/mind/conscience.t.md", home);
    let (a, t) = (action.to_string(), target.to_string());

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let out = Command::new(&bin)
            .args([
                "compile",
                &agent,
                "--run",
                "Conscience",
                "--set",
                &format!("action={}", a),
                "--set",
                &format!("target={}", t),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let _ = tx.send(out);
    });

    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(output)) => {
            let s = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            parse_refusal(&s)
        }
        _ => {
            eprintln!("glassbox: values rail unavailable — failing open");
            None
        }
    }
}

/// Parse `Refusal(reason=<q>...<q>, policy=<q>...<q>)` where <q> is ' or ".
/// Tessera's repr-style quoting guarantees the delimiter never appears inside,
/// so matching the next same-quote closes correctly.
fn parse_refusal(s: &str) -> Option<(String, String)> {
    let after_reason = &s[s.find("Refusal(reason=")? + "Refusal(reason=".len()..];
    let (reason, rest) = read_quoted(after_reason)?;
    let after_policy = &rest[rest.find("policy=")? + "policy=".len()..];
    let (policy, _) = read_quoted(after_policy)?;
    Some((reason, policy))
}

/// Read a quoted string at the start of `s`, returning (contents, remainder).
fn read_quoted(s: &str) -> Option<(String, &str)> {
    let q = s.chars().next()?;
    if q != '\'' && q != '"' {
        return None;
    }
    let body = &s[q.len_utf8()..];
    let end = body.find(q)?;
    Some((body[..end].to_string(), &body[end + q.len_utf8()..]))
}
