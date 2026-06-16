//! Audit trail + Nerve event stream. Best-effort; governance never fails because
//! logging did.

use crate::gate::Verdict;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn record(action: &str, target: &str, verdicts: &[Verdict], blocked: bool, reason: &str) {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let home = std::env::var("HOME").unwrap_or_default();

    let vlist: Vec<_> = verdicts
        .iter()
        .map(|v| serde_json::json!({"rail": v.rail, "refused": v.refused, "reason": v.reason}))
        .collect();
    let entry = serde_json::json!({
        "t": t, "action": action, "target": target,
        "verdicts": vlist, "blocked": blocked, "reason": reason,
    });

    let dir = format!("{}/.glassbox", home);
    let _ = create_dir_all(&dir);
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{}/decisions.jsonl", dir))
    {
        let _ = writeln!(f, "{}", entry);
    }

    // Nerve event stream — only if the bus dir exists.
    let nerve_dir = format!("{}/.claude/nerve", home);
    if std::path::Path::new(&nerve_dir).is_dir() {
        let ev = serde_json::json!({
            "t": t,
            "type": if blocked { "glassbox:blocked" } else { "glassbox:allowed" },
            "source": "glassbox",
            "data": entry,
        });
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{}/events.jsonl", nerve_dir))
        {
            let _ = writeln!(f, "{}", ev);
        }
    }
}
