//! The trust-card — the legible moment-of-action render.

use crate::gate::Verdict;

pub fn render(
    action: &str,
    target: &str,
    verdicts: &[Verdict],
    blocked: bool,
    reason: &str,
) -> String {
    let mut lines = vec![
        "  ┌─ GLASS BOX ─ moment of action ─────────────".to_string(),
        format!(
            "    perceive  {}",
            action.chars().take(60).collect::<String>()
        ),
        format!("    target    {}", target),
    ];
    for v in verdicts {
        if v.refused {
            lines.push(format!(
                "    {:<7} ⛔ refused  {}  [{}]",
                v.rail, v.reason, v.policy
            ));
        } else {
            lines.push(format!("    {:<7} ✓ clean", v.rail));
        }
    }
    let d = if blocked {
        "⛔ BLOCKED"
    } else {
        "✓ ALLOWED"
    };
    lines.push(format!("    decision  {}  {}", d, reason));
    lines.push("  └──────────────────────────────────────────".to_string());
    lines.join("\n")
}
