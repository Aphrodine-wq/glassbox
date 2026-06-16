//! The universal gate core.
//!
//! [`run_gate`] is the single place every entry point funnels through — the
//! Claude Code hook, the generic `gate-json` adapter, the human CLI, and the
//! demo. It evaluates both rails, decides, renders the card, synthesizes
//! provenance, mints a stable id, and records the decision. The request shape is
//! agent-agnostic (`{action, target, agent, mode}`) so anything that can describe
//! a proposed action can be governed — not just Claude Code.
//!
//! `decision` (what the mode allowed) is deliberately separate from `blocked`
//! (what the gate decided): `blocked:true, decision:"would-refuse"` is the entire
//! point of shadow.

use crate::gate::{self, Verdict};
use crate::mode::Mode;
use crate::provenance::{self, Provenance};
use crate::{audit, card};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct GateRequest {
    pub action: String,
    pub target: String,
    pub agent: String,
    pub mode: Mode,
    /// Link the values provenance to Tessera's governance graph (an extra,
    /// latency-tolerant read). Off for the hook hot path; on for interactive CLI.
    pub link: bool,
}

impl GateRequest {
    /// Parse the generic agent-agnostic request. Mode resolution: request field →
    /// `GLASSBOX_MODE` env → `Shadow`.
    pub fn from_json(v: &serde_json::Value) -> GateRequest {
        let s = |k: &str, default: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or(default)
                .to_string()
        };
        let mode = match v.get("mode").and_then(|x| x.as_str()) {
            Some(m) => Mode::resolve(Some(m)),
            None => Mode::from_env(),
        };
        GateRequest {
            action: s("action", ""),
            target: s("target", "unknown"),
            agent: s("agent", "unknown"),
            mode,
            link: v.get("link").and_then(|x| x.as_bool()).unwrap_or(false),
        }
    }
}

pub struct GateResponse {
    pub action: String,
    pub target: String,
    pub agent: String,
    pub mode: Mode,
    pub decision: String,
    pub blocked: bool,
    pub reason: String,
    pub verdicts: Vec<Verdict>,
    pub card: String,
    pub provenance: Option<Provenance>,
    pub provenance_id: String,
    pub t: u64,
}

impl GateResponse {
    /// The structured record (no card) — used by the audit log and as the base
    /// for the `gate-json` response.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "t": self.t,
            "action": self.action,
            "target": self.target,
            "agent": self.agent,
            "mode": self.mode.label(),
            "decision": self.decision,
            "blocked": self.blocked,
            "reason": self.reason,
            "verdicts": self.verdicts.iter().map(verdict_json).collect::<Vec<_>>(),
            "provenance": self.provenance.as_ref().map(prov_json),
            "provenance_id": self.provenance_id,
        })
    }

    /// The full `gate-json` response: the structured record plus the rendered card.
    pub fn to_json_with_card(&self) -> serde_json::Value {
        let mut v = self.to_value();
        v["card"] = serde_json::Value::String(self.card.clone());
        v
    }

    /// A one-line posture summary for the hook's `systemMessage` — the live wedge.
    /// The human sees the governed decision inline at the moment of action, even
    /// though shadow lets it proceed; the full card lives in `glassbox watch`.
    pub fn summary_line(&self) -> String {
        let posture = self.mode.label().to_uppercase();
        let verdict = if self.blocked {
            let policy = self
                .verdicts
                .iter()
                .find(|v| v.refused)
                .map(|v| v.policy.as_str())
                .unwrap_or("");
            if self.mode == Mode::Shadow {
                format!("WOULD-REFUSE ({policy})")
            } else {
                format!("BLOCKED ({policy})")
            }
        } else if self.mode == Mode::Shadow {
            "would-allow".to_string()
        } else {
            "allow".to_string()
        };
        let action_short: String = self.action.chars().take(48).collect();
        let rails = self
            .verdicts
            .iter()
            .map(|v| format!("{}{}", v.rail, if v.refused { "⛔" } else { "✓" }))
            .collect::<Vec<_>>()
            .join(" ");
        let tail = if self.blocked {
            " · see: glassbox watch"
        } else {
            ""
        };
        format!(
            "Glass Box · {posture} · {verdict} · {action_short} · {rails}{tail} · {}",
            self.provenance_id
        )
    }
}

fn verdict_json(v: &Verdict) -> serde_json::Value {
    serde_json::json!({
        "rail": v.rail,
        "refused": v.refused,
        "reason": v.reason,
        "policy": v.policy,
    })
}

fn prov_json(p: &Provenance) -> serde_json::Value {
    serde_json::json!({
        "source": p.source,
        "policy": p.policy,
        "value": p.value,
        "intent": p.intent,
        "escalation": p.escalation,
        "tessera_seq": p.tessera_seq,
        "tessera_created_at": p.tessera_created_at,
    })
}

/// The one true core. Evaluate (always both rails so the card is never
/// half-governed), decide, render, synthesize provenance, mint id, record.
pub fn run_gate(req: &GateRequest) -> GateResponse {
    let t = now_millis();
    let verdicts = gate::evaluate(&req.action, &req.target, true);
    let (blocked, reason) = gate::decide(&verdicts);

    let mut provenance = verdicts
        .iter()
        .find(|v| v.refused)
        .map(|v| provenance::synthesize(v, &req.action));
    if req.link {
        if let Some(p) = provenance.as_mut() {
            provenance::link_tessera(p); // off the hot path; best-effort
        }
    }

    let provenance_id = mint_id(t);
    let card = card::render(
        &req.action,
        &req.target,
        &verdicts,
        blocked,
        &reason,
        req.mode,
    );
    let decision = decision_label(req.mode, blocked);

    let resp = GateResponse {
        action: req.action.clone(),
        target: req.target.clone(),
        agent: req.agent.clone(),
        mode: req.mode,
        decision,
        blocked,
        reason,
        verdicts,
        card,
        provenance,
        provenance_id,
        t,
    };
    audit::record(&resp);
    resp
}

fn decision_label(mode: Mode, blocked: bool) -> String {
    match (mode, blocked) {
        (Mode::Shadow, true) => "would-refuse".into(),
        (Mode::Shadow, false) => "would-allow".into(),
        (Mode::Enforce, true) => "deny".into(),
        (Mode::Enforce, false) => "allow".into(),
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `gbx_<millis>_<counter:04x>` — dep-free, collision-free within a process.
fn mint_id(t: u64) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff;
    format!("gbx_{t}_{n:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_labels_track_mode_and_blocked() {
        assert_eq!(decision_label(Mode::Shadow, true), "would-refuse");
        assert_eq!(decision_label(Mode::Shadow, false), "would-allow");
        assert_eq!(decision_label(Mode::Enforce, true), "deny");
        assert_eq!(decision_label(Mode::Enforce, false), "allow");
    }

    #[test]
    fn mint_id_is_unique_and_prefixed() {
        let a = mint_id(1000);
        let b = mint_id(1000);
        assert!(a.starts_with("gbx_1000_"));
        assert_ne!(a, b); // counter advances
    }

    #[test]
    fn from_json_defaults_and_mode() {
        let req = GateRequest::from_json(&serde_json::json!({"action": "rm -rf /x"}));
        assert_eq!(req.action, "rm -rf /x");
        assert_eq!(req.target, "unknown");
        assert_eq!(req.agent, "unknown");
        assert!(!req.link);
        // explicit enforce in the request is honored
        let req = GateRequest::from_json(&serde_json::json!({"action": "x", "mode": "enforce"}));
        assert_eq!(req.mode, Mode::Enforce);
    }
}
