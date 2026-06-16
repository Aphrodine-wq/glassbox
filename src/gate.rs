//! The gate — perceive an action and run it through the two governed rails.
//!
//! Both rails answer different questions: `safety` refuses what is *irreversible*
//! (fast, in-process, never fails), `values` refuses what is *wrong* (governed by
//! the Conscience markdown, fail-open on infra error). Blocked if EITHER refuses.

use crate::{safety, values};

pub struct Verdict {
    pub rail: String,
    pub refused: bool,
    pub reason: String,
    pub policy: String,
}

/// Turn a Claude Code tool call into a (action, target) the rails can read.
pub fn perceive(tool_name: &str, tool_input: &serde_json::Value) -> (String, String) {
    match tool_name {
        "Bash" => (
            tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "shell".to_string(),
        ),
        "Edit" | "Write" | "NotebookEdit" => {
            let p = tool_input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (format!("{} {}", tool_name.to_lowercase(), p), p.to_string())
        }
        "Read" | "Glob" | "Grep" => {
            let p = tool_input
                .get("file_path")
                .and_then(|v| v.as_str())
                .or_else(|| tool_input.get("pattern").and_then(|v| v.as_str()))
                .unwrap_or("");
            (
                format!("{} {}", tool_name.to_lowercase(), p),
                "fs".to_string(),
            )
        }
        _ => {
            let blob: String = tool_input.to_string().chars().take(200).collect();
            (format!("{} {}", tool_name, blob), tool_name.to_string())
        }
    }
}

/// Run the rails. `full` = run both even if safety already refused (for the
/// demo/card). When `full` is false (the hook hot path) a safety refusal
/// short-circuits and skips the values subprocess.
pub fn evaluate(action: &str, target: &str, full: bool) -> Vec<Verdict> {
    evaluate_with(action, target, full, values::active_oracle())
}

/// `evaluate` with an injectable values oracle. Production uses the env-derived
/// Tessera oracle; tests inject a fake to prove the safety short-circuit never
/// consults the values rail.
pub fn evaluate_with(
    action: &str,
    target: &str,
    full: bool,
    oracle: &dyn values::ConscienceOracle,
) -> Vec<Verdict> {
    let s = safety::check(action);
    if s.refused && !full {
        return vec![s];
    }
    let v = values::check_with(action, target, oracle);
    vec![v, s]
}

/// Collapse rail verdicts: blocked if ANY rail refused.
pub fn decide(verdicts: &[Verdict]) -> (bool, String) {
    for v in verdicts {
        if v.refused {
            return (true, format!("{} rail: {}", v.rail, v.reason));
        }
    }
    (false, "all rails clean".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Panics if consulted — proves the safety short-circuit skips values.
    struct PanicOracle;
    impl values::ConscienceOracle for PanicOracle {
        fn consult(&self, _a: &str, _t: &str) -> Option<values::Refusal> {
            panic!("values oracle consulted during a safety short-circuit");
        }
    }

    struct CleanOracle;
    impl values::ConscienceOracle for CleanOracle {
        fn consult(&self, _a: &str, _t: &str) -> Option<values::Refusal> {
            None
        }
    }

    #[test]
    fn perceive_bash_is_the_command() {
        let (a, t) = perceive("Bash", &serde_json::json!({"command": "ls -la"}));
        assert_eq!(a, "ls -la");
        assert_eq!(t, "shell");
    }

    #[test]
    fn perceive_edit_and_write_carry_the_path() {
        let (a, t) = perceive("Edit", &serde_json::json!({"file_path": "/tmp/x.rs"}));
        assert_eq!(a, "edit /tmp/x.rs");
        assert_eq!(t, "/tmp/x.rs");
        let (a, t) = perceive("Write", &serde_json::json!({"file_path": "/tmp/y"}));
        assert_eq!(a, "write /tmp/y");
        assert_eq!(t, "/tmp/y");
    }

    #[test]
    fn perceive_read_falls_back_to_pattern() {
        let (a, t) = perceive("Read", &serde_json::json!({"file_path": "/etc/hosts"}));
        assert_eq!(a, "read /etc/hosts");
        assert_eq!(t, "fs");
        let (a, t) = perceive("Grep", &serde_json::json!({"pattern": "TODO"}));
        assert_eq!(a, "grep TODO");
        assert_eq!(t, "fs");
    }

    #[test]
    fn perceive_unknown_tool_truncates_blob() {
        let big = "x".repeat(500);
        let (a, t) = perceive("Mystery", &serde_json::json!({ "blob": big }));
        assert!(a.starts_with("Mystery "));
        assert!(a.chars().count() <= "Mystery ".len() + 200);
        assert_eq!(t, "Mystery");
    }

    #[test]
    fn decide_blocks_on_any_refusal_first_wins() {
        let verdicts = vec![
            Verdict {
                rail: "values".into(),
                refused: true,
                reason: "wrong".into(),
                policy: "NoExtraction".into(),
            },
            Verdict {
                rail: "safety".into(),
                refused: true,
                reason: "irreversible".into(),
                policy: "Irreversible".into(),
            },
        ];
        let (blocked, reason) = decide(&verdicts);
        assert!(blocked);
        assert!(reason.starts_with("values rail:")); // first refusal in the vec wins
    }

    #[test]
    fn decide_allows_when_all_clean() {
        let verdicts = vec![
            Verdict {
                rail: "values".into(),
                refused: false,
                reason: "clean".into(),
                policy: String::new(),
            },
            Verdict {
                rail: "safety".into(),
                refused: false,
                reason: "clean".into(),
                policy: String::new(),
            },
        ];
        let (blocked, reason) = decide(&verdicts);
        assert!(!blocked);
        assert_eq!(reason, "all rails clean");
    }

    #[test]
    fn evaluate_short_circuits_on_safety_refusal() {
        // rm -rf trips safety; full=false ⇒ values is skipped ⇒ the panic oracle
        // is never consulted (no panic = pass).
        let verdicts = evaluate_with("rm -rf /tmp/x", "shell", false, &PanicOracle);
        assert_eq!(verdicts.len(), 1);
        assert!(verdicts[0].refused);
        assert_eq!(verdicts[0].rail, "safety");
    }

    #[test]
    fn evaluate_runs_both_rails_when_safety_clean() {
        let verdicts = evaluate_with("git status", "shell", false, &CleanOracle);
        assert_eq!(verdicts.len(), 2);
        assert!(!verdicts[0].refused && !verdicts[1].refused);
    }
}
