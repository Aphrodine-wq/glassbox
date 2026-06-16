//! Audit trail + Nerve event stream. Best-effort; governance never fails because
//! logging did. `~/.glassbox/decisions.jsonl` is the system of record — each line
//! is the structured decision (verdicts + synthesized provenance + id + mode), so
//! `status` and `watch` can reconstruct the card without a separate store.

use crate::protocol::GateResponse;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;

pub fn record(resp: &GateResponse) {
    let home = std::env::var("HOME").unwrap_or_default();
    let entry = resp.to_value();

    let dir = format!("{home}/.glassbox");
    let _ = create_dir_all(&dir);
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{dir}/decisions.jsonl"))
    {
        let _ = writeln!(f, "{entry}");
    }

    // Nerve event stream — only if the bus dir exists.
    let nerve_dir = format!("{home}/.claude/nerve");
    if std::path::Path::new(&nerve_dir).is_dir() {
        let ev = serde_json::json!({
            "t": resp.t,
            "type": if resp.blocked { "glassbox:blocked" } else { "glassbox:allowed" },
            "source": "glassbox",
            "data": entry,
        });
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{nerve_dir}/events.jsonl"))
        {
            let _ = writeln!(f, "{ev}");
        }
    }
}
