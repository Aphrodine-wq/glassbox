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
    let s = safety::check(action);
    if s.refused && !full {
        return vec![s];
    }
    let v = values::check(action, target);
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
