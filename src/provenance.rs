//! Provenance — the *why* behind a decision.
//!
//! Synthesized in-process with **no I/O** so it is free on the hot path: the
//! refusing rail's verdict plus the framing already declared in the governance
//! `.t.md` files (which value, which intent, where it escalates). The safety rail
//! never touches Tessera, so its provenance is authored here. The values rail's
//! richer record — sequence number, timestamp — already lives in Tessera's
//! `audit_governance.db`; [`link_tessera`] back-fills it OFF the hot path (only
//! `status`/`eval` call it) via the existing `tessera audit query` CLI, so it can
//! never slow or block a tool call.

use crate::gate::Verdict;

#[derive(Clone, Debug)]
pub struct Provenance {
    pub source: String,     // "glassbox/safety" | "tessera/conscience"
    pub policy: String,     // "Irreversible" | "NoExtraction"
    pub value: String,      // "reversibility" | "fairness" | "loyalty" | "care"
    pub intent: String,     // "guard_action" | "gate_action"
    pub escalation: String, // "a human clears irreversible calls" | "payments" | "pricing" | "values"
    pub tessera_seq: Option<u64>,
    pub tessera_created_at: Option<String>,
}

/// Build provenance from a refused verdict and the action. Pure; no I/O.
pub fn synthesize(v: &Verdict, action: &str) -> Provenance {
    let a = action.to_lowercase();
    let contains_any = |ks: &[&str]| ks.iter().any(|k| a.contains(k));

    if v.rail == "safety" {
        return Provenance {
            source: "glassbox/safety".into(),
            policy: non_empty(&v.policy, "Irreversible"),
            value: "reversibility".into(),
            intent: "guard_action".into(),
            escalation: "a human clears irreversible calls".into(),
            tessera_seq: None,
            tessera_created_at: None,
        };
    }

    // values / conscience — map the action to the moral foundation it offends.
    // Loyalty is checked first: repricing a loyal client (the loyalty rule) is a
    // loyalty violation even though "reprice" also reads as a fairness keyword.
    let value = if contains_any(&["loyal", "abandon", "betray"]) {
        "loyalty"
    } else if contains_any(&[
        "reprice", "gouge", "squeeze", "exploit", "defraud", "markup", "upcharge",
    ]) {
        "fairness"
    } else if contains_any(&["harm", "endanger", "deceive"]) {
        "care"
    } else {
        "fairness"
    };
    // Where conscience's autonomy block says it escalates to James.
    let escalation = if contains_any(&["charge", "invoice", "deposit", "refund", "payment"]) {
        "payments"
    } else if contains_any(&["reprice", "price", "pricing", "markup", "discount"]) {
        "pricing"
    } else {
        "values"
    };

    Provenance {
        source: "tessera/conscience".into(),
        policy: non_empty(&v.policy, "NoExtraction"),
        value: value.into(),
        intent: "gate_action".into(),
        escalation: escalation.into(),
        tessera_seq: None,
        tessera_created_at: None,
    }
}

fn non_empty(s: &str, fallback: &str) -> String {
    if s.is_empty() {
        fallback.to_string()
    } else {
        s.to_string()
    }
}

/// Off the hot path only (`status`/`eval`). Best-effort: back-fill the Tessera
/// governance sequence + timestamp from the existing query CLI. Never blocks the
/// gate; any failure leaves the fields `None` (the link is cosmetic).
pub fn link_tessera(p: &mut Provenance) {
    if !p.source.contains("conscience") {
        return; // only the values rail has a Tessera governance record
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let bin = std::env::var("GLASSBOX_TESSERA_BIN")
        .unwrap_or_else(|_| format!("{home}/Projects/walt/tessera/.venv/bin/tessera"));

    let out = std::process::Command::new(&bin)
        .args([
            "audit",
            "query",
            "--agent",
            "Conscience",
            "--action",
            "refusal",
            "--tier",
            "governance",
            "--limit",
            "1",
        ])
        .output();

    if let Ok(o) = out {
        let text = String::from_utf8_lossy(&o.stdout);
        if let Some(line) = text.lines().filter(|l| !l.trim().is_empty()).next_back() {
            if let Ok(row) = serde_json::from_str::<serde_json::Value>(line) {
                p.tessera_seq = row.get("seq").and_then(|v| v.as_u64());
                p.tessera_created_at = row
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(rail: &str, policy: &str) -> Verdict {
        Verdict {
            rail: rail.into(),
            refused: true,
            reason: "r".into(),
            policy: policy.into(),
        }
    }

    #[test]
    fn safety_provenance_is_reversibility() {
        let p = synthesize(&refused("safety", "Irreversible"), "rm -rf /x");
        assert_eq!(p.source, "glassbox/safety");
        assert_eq!(p.value, "reversibility");
        assert_eq!(p.intent, "guard_action");
        assert!(p.escalation.contains("human"));
        assert!(p.tessera_seq.is_none());
    }

    #[test]
    fn repricing_loyal_client_is_loyalty_and_pricing() {
        let p = synthesize(
            &refused("values", "NoExtraction"),
            "reprice loyal client to market",
        );
        assert_eq!(p.source, "tessera/conscience");
        assert_eq!(p.value, "loyalty"); // loyalty rule beats the bare "reprice" fairness read
        assert_eq!(p.escalation, "pricing");
        assert_eq!(p.intent, "gate_action");
    }

    #[test]
    fn charging_a_payment_escalates_to_payments() {
        let p = synthesize(
            &refused("values", "NoExtraction"),
            "charge the homeowner deposit invoice",
        );
        assert_eq!(p.escalation, "payments");
    }

    #[test]
    fn gouging_a_stranger_is_fairness() {
        let p = synthesize(
            &refused("values", "NoExtraction"),
            "gouge the homeowner on markup",
        );
        assert_eq!(p.value, "fairness");
    }

    #[test]
    fn empty_policy_falls_back_per_rail() {
        assert_eq!(
            synthesize(&refused("safety", ""), "rm -rf /x").policy,
            "Irreversible"
        );
        assert_eq!(
            synthesize(&refused("values", ""), "gouge them").policy,
            "NoExtraction"
        );
    }
}
