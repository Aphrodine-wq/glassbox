//! The envelope rail — bounds *autonomy*: blast-radius, budget, reversibility,
//! and the reserved denylist.
//!
//! Where `safety` refuses the irreversible and `values` refuses the wrong, the
//! envelope refuses the *out-of-bounds*: a target outside the agent's allowed
//! scope, a host it may not reach, a budget it has spent, or a path that is one
//! of the rules-that-watch-it (reserved). Runs fully in-process (no subprocess).
//!
//! FAIL-OPEN by default: with no governance file loaded, every action passes —
//! so adding this rail does NOT change the live hook's behaviour. It only bites
//! once a governance file (`GLASSBOX_ENVELOPE_FILE` or `glassbox/governance.json`)
//! is present. Opting into governance always re-arms the reserved floor.

use crate::gate::Verdict;
use std::fs;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

struct EnvelopeConfig {
    allowed_paths: Vec<String>, // ~ expanded; if non-empty, file targets must be inside one
    allowed_hosts: Vec<String>, // if a host is referenced, it must be in this list
    reserved: Vec<String>,      // substrings that may never be touched (the hard invariant)
    max_actions_per_hour: u64,  // 0 = unbounded
}

// Re-armed whenever a governance file is loaded, even if it omits "reserved".
fn reserved_floor() -> Vec<String> {
    [
        "verify.py",
        "conscience",
        "brain_gate",
        "autonomy.js",
        "autonomy-grants",
        "run_evals",
        "gate-mode",
        "values-roster",
        "envelope.rs",
        "governance.json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn load_config() -> Option<EnvelopeConfig> {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::env::var("GLASSBOX_ENVELOPE_FILE")
        .unwrap_or_else(|_| format!("{home}/Projects/walt/glassbox/governance.json"));
    let text = fs::read_to_string(&path).ok()?; // absent => None => fail-open
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;

    let strs = |k: &str, expand: bool| -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .map(|s| {
                        if expand && s.starts_with('~') {
                            s.replacen('~', &home, 1)
                        } else {
                            s.to_string()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut reserved = strs("reserved", false);
    if reserved.is_empty() {
        reserved = reserved_floor(); // governance-on always re-arms the floor
    }
    Some(EnvelopeConfig {
        allowed_paths: strs("allowed_paths", true),
        allowed_hosts: strs("allowed_hosts", false),
        reserved,
        max_actions_per_hour: v
            .get("max_actions_per_hour")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
    })
}

fn config() -> Option<&'static EnvelopeConfig> {
    static CACHE: OnceLock<Option<EnvelopeConfig>> = OnceLock::new();
    CACHE.get_or_init(load_config).as_ref()
}

fn clean() -> Verdict {
    Verdict {
        rail: "envelope".into(),
        refused: false,
        reason: "clean".into(),
        policy: String::new(),
    }
}
fn refuse(policy: &str, reason: String) -> Verdict {
    Verdict {
        rail: "envelope".into(),
        refused: true,
        reason,
        policy: policy.into(),
    }
}

/// First filesystem-ish path referenced in the text (a token starting with `/`
/// or `~/`). `~` is expanded so it can be compared to the allowed roots.
fn referenced_path(text: &str, home: &str) -> Option<String> {
    for raw in text.split_whitespace() {
        let tok = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
        if tok.starts_with('/') && tok.len() > 1 {
            return Some(tok.to_string());
        }
        if let Some(rest) = tok.strip_prefix("~/") {
            return Some(format!("{home}/{rest}"));
        }
    }
    None
}

/// First network host referenced: the authority after `://`, or a bare
/// domain-looking token (has a dot, no slash).
fn referenced_host(text: &str) -> Option<String> {
    if let Some(i) = text.find("://") {
        let after = &text[i + 3..];
        let host: String = after
            .chars()
            .take_while(|c| !matches!(c, '/' | '"' | '\'' | ' ' | ':'))
            .collect();
        if !host.is_empty() {
            return Some(host);
        }
    }
    for raw in text.split_whitespace() {
        let tok = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
        if tok.contains('.') && !tok.contains('/') && !tok.starts_with('~') && !tok.starts_with('.')
        {
            // crude domain check: label.tld, no path separators
            if tok.split('.').all(|p| !p.is_empty()) && tok.split('.').count() >= 2 {
                return Some(tok.to_string());
            }
        }
    }
    None
}

fn actions_last_hour() -> u64 {
    let home = std::env::var("HOME").unwrap_or_default();
    let ledger = format!("{home}/.claude/nerve/autonomy-ledger.jsonl");
    let text = match fs::read_to_string(&ledger) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let cutoff = now.saturating_sub(3_600_000);
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e.get("t").and_then(|v| v.as_u64()).unwrap_or(0) >= cutoff)
        .count() as u64
}

pub fn check(action: &str, target: &str) -> Verdict {
    let cfg = match config() {
        Some(c) => c,
        None => return clean(), // fail-open: no governance => permissive
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let combined = format!("{action} {target}");

    // 1. reserved (the hard invariant) — never touch the rules that watch the agent.
    for r in &cfg.reserved {
        if action.contains(r.as_str()) || target.contains(r.as_str()) {
            return refuse("Reserved", format!("touches reserved path '{r}'"));
        }
    }
    // 2. blast-radius (paths) — a filesystem target must live inside an allowed root.
    if !cfg.allowed_paths.is_empty() {
        if let Some(p) = referenced_path(&combined, &home) {
            if !cfg
                .allowed_paths
                .iter()
                .any(|root| p.starts_with(root.as_str()))
            {
                return refuse(
                    "OutOfScope",
                    format!("path '{p}' is outside the allowed scope"),
                );
            }
        }
    }
    // 3. blast-radius (hosts) — any referenced host must be on the allowlist.
    if let Some(host) = referenced_host(&combined) {
        if !cfg.allowed_hosts.iter().any(|h| h == &host) {
            return refuse(
                "OutOfScope",
                format!("host '{host}' is not on the allowed list"),
            );
        }
    }
    // 4. budget — actions/hour cap (reads the shared autonomy ledger).
    if cfg.max_actions_per_hour > 0 && actions_last_hour() >= cfg.max_actions_per_hour {
        return refuse(
            "BudgetExceeded",
            format!("over {} actions/hour", cfg.max_actions_per_hour),
        );
    }
    clean()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_open_with_no_governance_file() {
        let _g = crate::test_support::ENV_LOCK.lock().unwrap();
        std::env::set_var("GLASSBOX_ENVELOPE_FILE", "/nonexistent-governance.json");
        // config() is cached, so test the loader directly for determinism.
        assert!(
            load_config().is_none(),
            "missing file => no config => fail-open"
        );
        std::env::remove_var("GLASSBOX_ENVELOPE_FILE");
    }

    #[test]
    fn referenced_path_finds_home_and_abs() {
        assert_eq!(
            referenced_path("read ~/.ssh/id_rsa", "/Users/x"),
            Some("/Users/x/.ssh/id_rsa".into())
        );
        assert_eq!(
            referenced_path("cat /etc/passwd", "/Users/x"),
            Some("/etc/passwd".into())
        );
        assert_eq!(referenced_path("read fs", "/Users/x"), None);
    }

    #[test]
    fn referenced_host_finds_url_and_bare() {
        assert_eq!(
            referenced_host("GET https://evil.example/exfil"),
            Some("evil.example".into())
        );
        assert_eq!(
            referenced_host("http_get evil.example"),
            Some("evil.example".into())
        );
        assert_eq!(referenced_host("read a file"), None);
    }
}
