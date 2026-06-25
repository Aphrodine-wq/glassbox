//! The verify subcommand — behavioural verification: did the agent do EXACTLY
//! what it declared?
//!
//! Ported from `hands/verify.py`, generalized to an agent-agnostic JSON contract
//! `{agent, declared, trace}` (the Tessera-specific regex parsing is dropped —
//! the JSON arrives pre-parsed). The property frontier pre-approval gates lack:
//! `safety`/`values`/`envelope` decide whether an action is ALLOWED; verify
//! decides whether what actually ran is what was DECLARED. Fail-closed.
//!
//!   declared = { "tools": ["t1", ..], "plan": [{"tool":"t1","arg":"\"x\""}, ..] }
//!   trace    = [ {"action":"tool:t1", ..}, .. ]

use serde_json::Value;

const ALLOWED_KEYS: [&str; 3] = ["tab", "return", "enter"];

pub struct Verdict {
    pub verified: bool,
    pub findings: Vec<String>,
}

/// Mirrors the verify.py SENSITIVE regex (credential-shaped literals), minus the
/// bare "pass" to avoid false positives on words like "passed".
fn credential_shaped(s: &str) -> bool {
    let l = s.to_lowercase();
    [
        "password",
        "passphrase",
        "secret",
        "api_key",
        "api-key",
        "apikey",
        "token",
        "ssn",
        "seed phrase",
        "seed_phrase",
        "seed-phrase",
    ]
    .iter()
    .any(|k| l.contains(k))
}

pub fn verify(declared: &Value, trace: &Value) -> Verdict {
    let mut findings: Vec<String> = Vec::new();

    let tools: Vec<String> = declared
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let plan = declared
        .get("plan")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if plan.is_empty() {
        findings.push("no plan calls found".into());
    }

    // Prove the DECLARED plan stays inside the boundary.
    let mut plan_seq: Vec<String> = Vec::new();
    for call in &plan {
        let tool = call.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        let arg = call.get("arg").and_then(|v| v.as_str()).unwrap_or("");
        plan_seq.push(tool.to_string());
        if !tools.is_empty() && !tools.iter().any(|t| t == tool) {
            findings.push(format!("plan calls undeclared tool '{tool}'"));
        }
        if tool == "press_key" {
            let lit = arg.trim_matches('"');
            if !lit.is_empty() && !ALLOWED_KEYS.contains(&lit) {
                findings.push(format!("press_key uses non-navigation key '{lit}'"));
            }
        }
        if credential_shaped(arg) {
            findings.push("credential-shaped literal baked into plan".into());
        }
    }

    // Prove what ACTUALLY ran matches what was declared, exactly and in order.
    let trace_seq: Vec<String> = trace
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("action").and_then(|v| v.as_str()))
                .filter(|act| act.starts_with("tool:"))
                .map(|act| act["tool:".len()..].to_string())
                .collect()
        })
        .unwrap_or_default();

    if trace_seq.is_empty() {
        findings.push("no audit trace to verify against — the plan never ran".into());
    } else if trace_seq != plan_seq {
        findings.push(format!(
            "trace diverges from declared plan: ran {trace_seq:?} vs declared {plan_seq:?}"
        ));
    }

    Verdict {
        verified: findings.is_empty(),
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matching_trace_verifies() {
        let declared = json!({"tools":["type_text","press_key"],
            "plan":[{"tool":"type_text","arg":"\"hi\""},{"tool":"press_key","arg":"\"tab\""}]});
        let trace = json!([{"action":"tool:type_text"},{"action":"tool:press_key"}]);
        let v = verify(&declared, &trace);
        assert!(v.verified, "{:?}", v.findings);
    }

    #[test]
    fn divergent_trace_is_unverified() {
        let declared =
            json!({"tools":["read_file"],"plan":[{"tool":"read_file","arg":"\"notes\""}]});
        // agent actually ran an extra exfil call it never declared
        let trace = json!([{"action":"tool:read_file"},{"action":"tool:http_get"}]);
        let v = verify(&declared, &trace);
        assert!(!v.verified);
        assert!(v.findings.iter().any(|f| f.contains("diverges")));
    }

    #[test]
    fn credential_and_bad_key_are_findings() {
        let declared = json!({"tools":["type_text","press_key"],
            "plan":[{"tool":"type_text","arg":"\"my password is hunter2\""},{"tool":"press_key","arg":"\"cmd\""}]});
        let trace = json!([{"action":"tool:type_text"},{"action":"tool:press_key"}]);
        let v = verify(&declared, &trace);
        assert!(!v.verified);
        assert!(v.findings.iter().any(|f| f.contains("credential")));
        assert!(v.findings.iter().any(|f| f.contains("non-navigation")));
    }

    #[test]
    fn empty_trace_fails_closed() {
        let declared = json!({"tools":["x"],"plan":[{"tool":"x","arg":""}]});
        let v = verify(&declared, &json!([]));
        assert!(!v.verified);
    }
}
