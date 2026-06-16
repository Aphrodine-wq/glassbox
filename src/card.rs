//! The trust-card — the legible moment-of-action render.
//!
//! The card states the posture (`[SHADOW]` vs `[ENFORCE]`), each rail's verdict,
//! the *why* under any refusal (value · intent · escalation), and the decision.
//! In shadow the decision reads `WOULD-REFUSE … — allowed to proceed (shadow)`:
//! the human sees the governed decision AND that shadow let it run. That contrast
//! is the whole thesis.

use crate::gate::Verdict;
use crate::mode::Mode;
use crate::provenance;

pub fn render(
    action: &str,
    target: &str,
    verdicts: &[Verdict],
    blocked: bool,
    reason: &str,
    mode: Mode,
) -> String {
    let shadow = mode == Mode::Shadow;
    let tag = if shadow { "[SHADOW]" } else { "[ENFORCE]" };
    let mut lines = vec![
        format!("  ┌─ GLASS BOX ─ moment of action ─ {} ─", tag),
        format!(
            "    perceive  {}",
            action.chars().take(60).collect::<String>()
        ),
        format!("    target    {}", target),
    ];
    for v in verdicts {
        if v.refused {
            let verb = if shadow { "would-refuse" } else { "refused" };
            lines.push(format!(
                "    {:<7} ⛔ {}  {}  [{}]",
                v.rail, verb, v.reason, v.policy
            ));
            let p = provenance::synthesize(v, action);
            lines.push(format!(
                "            ↳ value: {} · intent: {} · escalate: {}",
                p.value, p.intent, p.escalation
            ));
        } else {
            lines.push(format!("    {:<7} ✓ clean", v.rail));
        }
    }
    let decision = match (shadow, blocked) {
        (true, true) => {
            let policy = verdicts
                .iter()
                .find(|v| v.refused)
                .map(|v| v.policy.as_str())
                .unwrap_or("");
            format!("SHADOW · WOULD-REFUSE ({policy}) — allowed to proceed (shadow)")
        }
        (true, false) => "SHADOW · would-allow".to_string(),
        (false, true) => format!("⛔ BLOCKED  {reason}"),
        (false, false) => format!("✓ ALLOWED  {reason}"),
    };
    lines.push(format!("    decision  {decision}"));
    lines.push("  └──────────────────────────────────────────".to_string());
    lines.join("\n")
}
