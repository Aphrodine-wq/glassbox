//! SQLite backend for Glassbox.
//!
//! Tables:
//!   decisions    -- every governed action, the core audit trail
//!   costs        -- token/spend events per agent
//!   api_keys     -- bearer tokens for ingest auth
//!   budgets      -- spend limits per agent
//!   webhooks     -- notification endpoints
//!   agent_config -- per-agent settings
//!   policies     -- governance policy templates
//!   retention    -- retention policy config
//!
//! On first run, migrates the schema and imports any existing JSONL data.

use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Db = Arc<Mutex<Connection>>;

// ---------------------------------------------------------------------------
// Budget check result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BudgetStatus {
    pub agent: String,
    pub max_usd: f64,
    pub spent_usd: f64,
    pub period: String,
    pub pct_used: f64,
    pub exceeded: bool,
    pub alert: bool,
}

// ---------------------------------------------------------------------------
// Database path & open
// ---------------------------------------------------------------------------

pub fn db_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(&home).join(".glassbox");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("glassbox.db")
}

pub fn open() -> Db {
    let path = db_path();
    let conn = Connection::open(&path).expect("failed to open glassbox.db");
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .expect("pragma");
    migrate(&conn);
    import_jsonl(&conn);
    Arc::new(Mutex::new(conn))
}

// ---------------------------------------------------------------------------
// Schema migration
// ---------------------------------------------------------------------------

fn migrate(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS decisions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            t           INTEGER NOT NULL,
            agent       TEXT NOT NULL,
            action      TEXT NOT NULL,
            target      TEXT NOT NULL DEFAULT '',
            decision    TEXT NOT NULL,
            blocked     INTEGER NOT NULL DEFAULT 0,
            reason      TEXT NOT NULL DEFAULT '',
            mode        TEXT NOT NULL DEFAULT 'shadow',
            provenance_id TEXT NOT NULL DEFAULT '',
            verdicts    TEXT NOT NULL DEFAULT '[]',
            provenance  TEXT,
            org         TEXT NOT NULL DEFAULT 'default',
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_decisions_agent ON decisions(agent);
        CREATE INDEX IF NOT EXISTS idx_decisions_t ON decisions(t);
        CREATE INDEX IF NOT EXISTS idx_decisions_org ON decisions(org);
        CREATE INDEX IF NOT EXISTS idx_decisions_blocked ON decisions(blocked);

        CREATE TABLE IF NOT EXISTS costs (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            t           INTEGER NOT NULL,
            agent       TEXT NOT NULL,
            tokens_in   INTEGER NOT NULL DEFAULT 0,
            tokens_out  INTEGER NOT NULL DEFAULT 0,
            cost_usd    REAL NOT NULL DEFAULT 0.0,
            model       TEXT NOT NULL DEFAULT '',
            org         TEXT NOT NULL DEFAULT 'default',
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_costs_agent ON costs(agent);

        CREATE TABLE IF NOT EXISTS api_keys (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            key_hash    TEXT NOT NULL UNIQUE,
            label       TEXT NOT NULL DEFAULT '',
            org         TEXT NOT NULL DEFAULT 'default',
            scopes      TEXT NOT NULL DEFAULT 'ingest',
            active      INTEGER NOT NULL DEFAULT 1,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- budgets: spend limits per agent
        CREATE TABLE IF NOT EXISTS budgets (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            agent           TEXT NOT NULL,
            max_usd         REAL NOT NULL,
            period          TEXT NOT NULL DEFAULT 'monthly',
            alert_at_pct    REAL NOT NULL DEFAULT 80.0,
            kill_on_exceed  INTEGER NOT NULL DEFAULT 0,
            org             TEXT NOT NULL DEFAULT 'default',
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_budgets_agent ON budgets(agent);

        -- webhooks: notification endpoints
        CREATE TABLE IF NOT EXISTS webhooks (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            url         TEXT NOT NULL,
            events      TEXT NOT NULL DEFAULT 'block',
            org         TEXT NOT NULL DEFAULT 'default',
            active      INTEGER NOT NULL DEFAULT 1,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_webhooks_org ON webhooks(org);

        -- agent_config: per-agent settings
        CREATE TABLE IF NOT EXISTS agent_config (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            agent               TEXT NOT NULL UNIQUE,
            mode                TEXT NOT NULL DEFAULT 'shadow',
            killed              INTEGER NOT NULL DEFAULT 0,
            kill_reason         TEXT NOT NULL DEFAULT '',
            group_name          TEXT NOT NULL DEFAULT '',
            team                TEXT NOT NULL DEFAULT '',
            environment         TEXT NOT NULL DEFAULT 'development',
            last_seen           INTEGER NOT NULL DEFAULT 0,
            health_timeout_secs INTEGER NOT NULL DEFAULT 300,
            org                 TEXT NOT NULL DEFAULT 'default'
        );

        CREATE INDEX IF NOT EXISTS idx_agent_config_agent ON agent_config(agent);

        -- policies: governance policy templates
        CREATE TABLE IF NOT EXISTS policies (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            rules       TEXT NOT NULL DEFAULT '[]',
            template    TEXT NOT NULL DEFAULT '',
            active      INTEGER NOT NULL DEFAULT 1,
            org         TEXT NOT NULL DEFAULT 'default',
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- retention: retention policy config
        CREATE TABLE IF NOT EXISTS retention (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            org          TEXT NOT NULL DEFAULT 'default',
            retain_days  INTEGER NOT NULL DEFAULT 365,
            archive_path TEXT NOT NULL DEFAULT '',
            created_at   TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )
    .expect("migration failed");
}

// ---------------------------------------------------------------------------
// JSONL import (one-time, on first run)
// ---------------------------------------------------------------------------

/// One-time import of existing JSONL files into SQLite.
/// Skips if decisions table already has data.
fn import_jsonl(conn: &Connection) {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
        .unwrap_or(0);
    if count > 0 {
        return; // already imported
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let decisions_path = format!("{home}/.glassbox/decisions.jsonl");
    let costs_path = format!("{home}/.glassbox/costs.jsonl");

    if let Ok(text) = std::fs::read_to_string(&decisions_path) {
        let mut imported = 0;
        for line in text.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            insert_decision_from_json(conn, &v, "default");
            imported += 1;
        }
        if imported > 0 {
            eprintln!("  imported {imported} decisions from JSONL");
        }
    }

    if let Ok(text) = std::fs::read_to_string(&costs_path) {
        let mut imported = 0;
        for line in text.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            insert_cost_from_json(conn, &v, "default");
            imported += 1;
        }
        if imported > 0 {
            eprintln!("  imported {imported} cost events from JSONL");
        }
    }
}

// ---------------------------------------------------------------------------
// Decision & cost insertion
// ---------------------------------------------------------------------------

pub fn insert_decision_from_json(conn: &Connection, v: &serde_json::Value, org: &str) {
    let t = v.get("t").and_then(|x| x.as_i64()).unwrap_or(0);
    let agent = v.get("agent").and_then(|x| x.as_str()).unwrap_or("unknown");
    let action = v.get("action").and_then(|x| x.as_str()).unwrap_or("");
    let target = v.get("target").and_then(|x| x.as_str()).unwrap_or("");
    let decision = v.get("decision").and_then(|x| x.as_str()).unwrap_or("");
    let blocked = v.get("blocked").and_then(|x| x.as_bool()).unwrap_or(false) as i32;
    let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("");
    let mode = v.get("mode").and_then(|x| x.as_str()).unwrap_or("shadow");
    let provenance_id = v
        .get("provenance_id")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let verdicts = v
        .get("verdicts")
        .map(|x| x.to_string())
        .unwrap_or_else(|| "[]".into());
    let provenance = v.get("provenance").map(|x| x.to_string());

    let _ = conn.execute(
        "INSERT INTO decisions (t, agent, action, target, decision, blocked, reason, mode, provenance_id, verdicts, provenance, org)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![t, agent, action, target, decision, blocked, reason, mode, provenance_id, verdicts, provenance, org],
    );

    // Update agent last-seen timestamp
    update_agent_last_seen(conn, agent, t);

    // If the decision was blocked, fire webhooks
    if blocked == 1 {
        let payload = serde_json::json!({
            "event": "block",
            "agent": agent,
            "action": action,
            "target": target,
            "reason": reason,
            "t": t,
            "org": org,
        });
        fire_webhooks(conn, "block", &payload.to_string());
    }
}

pub fn insert_cost_from_json(conn: &Connection, v: &serde_json::Value, org: &str) {
    let t = v.get("t").and_then(|x| x.as_i64()).unwrap_or(0);
    let agent = v.get("agent").and_then(|x| x.as_str()).unwrap_or("unknown");
    let tokens_in = v.get("tokens_in").and_then(|x| x.as_i64()).unwrap_or(0);
    let tokens_out = v.get("tokens_out").and_then(|x| x.as_i64()).unwrap_or(0);
    let cost_usd = v.get("cost_usd").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let model = v.get("model").and_then(|x| x.as_str()).unwrap_or("");

    let _ = conn.execute(
        "INSERT INTO costs (t, agent, tokens_in, tokens_out, cost_usd, model, org)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![t, agent, tokens_in, tokens_out, cost_usd, model, org],
    );

    // Check budget after inserting cost
    if let Some(status) = check_budget(conn, agent, org) {
        if status.exceeded {
            let payload = serde_json::json!({
                "event": "budget_exceed",
                "agent": agent,
                "max_usd": status.max_usd,
                "spent_usd": status.spent_usd,
                "pct_used": status.pct_used,
                "period": status.period,
                "org": org,
            });
            fire_webhooks(conn, "budget_exceed", &payload.to_string());
        } else if status.alert {
            let payload = serde_json::json!({
                "event": "budget_alert",
                "agent": agent,
                "max_usd": status.max_usd,
                "spent_usd": status.spent_usd,
                "pct_used": status.pct_used,
                "period": status.period,
                "org": org,
            });
            fire_webhooks(conn, "budget_alert", &payload.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// API key management
// ---------------------------------------------------------------------------

/// Generate an API key: returns (raw_key, hash).
pub fn create_api_key(conn: &Connection, label: &str, org: &str) -> String {
    let raw = format!("gbx_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
    let hash = hash_key(&raw);
    conn.execute(
        "INSERT INTO api_keys (key_hash, label, org) VALUES (?1, ?2, ?3)",
        params![hash, label, org],
    )
    .expect("insert api key");
    raw
}

pub fn hash_key(raw: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut h = Sha256::new();
    h.update(raw.as_bytes());
    hex::encode(h.finalize())
}

/// Validate a bearer token. Returns the org if valid.
pub fn validate_key(conn: &Connection, raw: &str) -> Option<String> {
    let hash = hash_key(raw);
    conn.query_row(
        "SELECT org FROM api_keys WHERE key_hash = ?1 AND active = 1",
        params![hash],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

// ---------------------------------------------------------------------------
// Budget helpers
// ---------------------------------------------------------------------------

/// Check if an agent is over budget. Returns `Some(BudgetStatus)` if the
/// agent has a budget configured, `None` otherwise.
pub fn check_budget(conn: &Connection, agent: &str, org: &str) -> Option<BudgetStatus> {
    let row: Option<(f64, String, f64)> = conn
        .query_row(
            "SELECT max_usd, period, alert_at_pct FROM budgets
             WHERE agent = ?1 AND org = ?2
             ORDER BY id DESC LIMIT 1",
            params![agent, org],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();

    let (max_usd, period, alert_at_pct) = row?;

    // Compute the start-of-period timestamp (unix seconds)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let period_start = match period.as_str() {
        "daily" => now - 86_400,
        "weekly" => now - 7 * 86_400,
        _ => now - 30 * 86_400, // monthly (default)
    };

    let spent_usd: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM costs
             WHERE agent = ?1 AND org = ?2 AND t >= ?3",
            params![agent, org, period_start],
            |r| r.get(0),
        )
        .unwrap_or(0.0);

    let pct_used = if max_usd > 0.0 {
        (spent_usd / max_usd) * 100.0
    } else {
        0.0
    };
    let exceeded = spent_usd >= max_usd;
    let alert = pct_used >= alert_at_pct && !exceeded;

    Some(BudgetStatus {
        agent: agent.to_string(),
        max_usd,
        spent_usd,
        period,
        pct_used,
        exceeded,
        alert,
    })
}

// ---------------------------------------------------------------------------
// Webhook helpers
// ---------------------------------------------------------------------------

/// Fire webhooks matching `event_type`. Each POST is spawned in its own
/// thread so we never block the caller.
pub fn fire_webhooks(conn: &Connection, event_type: &str, payload_json: &str) {
    // Collect matching webhook URLs
    let mut stmt = match conn.prepare(
        "SELECT url FROM webhooks WHERE active = 1",
    ) {
        Ok(s) => s,
        Err(_) => return,
    };

    let event_owned = event_type.to_string();
    let urls: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .ok()
        .map(|rows| {
            rows.filter_map(|r| r.ok())
                .filter(|_url| true) // we filter below after collecting
                .collect()
        })
        .unwrap_or_default();

    let payload_owned = payload_json.to_string();

    for url in urls {
        // Check if this webhook's events list matches the event_type
        let events_str: String = conn
            .query_row(
                "SELECT events FROM webhooks WHERE url = ?1 AND active = 1 LIMIT 1",
                params![url],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "block".to_string());

        let events: Vec<&str> = events_str.split(',').map(|s| s.trim()).collect();
        if !events.contains(&"all") && !events.contains(&event_owned.as_str()) {
            continue;
        }

        let url_clone = url.clone();
        let payload_clone = payload_owned.clone();
        // Fire-and-forget in a background thread
        std::thread::spawn(move || {
            // Best-effort HTTP POST using a minimal approach.
            // We use a blocking TCP connection since we don't want to pull in
            // an async HTTP client just for webhooks.
            let _ = post_webhook(&url_clone, &payload_clone);
        });
    }
}

/// Minimal HTTP POST for webhook delivery. Fire-and-forget; errors are
/// silently ignored.
fn post_webhook(url: &str, body: &str) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;
    use std::net::TcpStream;

    // Parse URL (very basic: http://host:port/path)
    let url = url.trim();
    let without_scheme = if let Some(rest) = url.strip_prefix("https://") {
        // We don't support TLS in this minimal implementation; skip silently
        let _ = rest;
        return Ok(());
    } else if let Some(rest) = url.strip_prefix("http://") {
        rest
    } else {
        return Ok(());
    };

    let (host_port, path) = match without_scheme.find('/') {
        Some(i) => (&without_scheme[..i], &without_scheme[i..]),
        None => (without_scheme, "/"),
    };

    let (host, port) = if let Some(i) = host_port.find(':') {
        (&host_port[..i], host_port[i + 1..].parse::<u16>().unwrap_or(80))
    } else {
        (host_port, 80u16)
    };

    let mut stream = TcpStream::connect((host, port))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path, host, body.len(), body
    );
    stream.write_all(request.as_bytes())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Agent config helpers
// ---------------------------------------------------------------------------

/// Upsert agent_config.last_seen for the given agent.
pub fn update_agent_last_seen(conn: &Connection, agent: &str, t: i64) {
    // Try to update first; if no row existed, insert.
    let updated = conn
        .execute(
            "UPDATE agent_config SET last_seen = ?1 WHERE agent = ?2",
            params![t, agent],
        )
        .unwrap_or(0);

    if updated == 0 {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO agent_config (agent, last_seen) VALUES (?1, ?2)",
            params![agent, t],
        );
    }
}

/// Returns agents whose `last_seen` is older than their configured
/// `health_timeout_secs`.
pub fn get_silent_agents(conn: &Connection, threshold_secs: i64) -> Vec<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut stmt = match conn.prepare(
        "SELECT agent FROM agent_config
         WHERE last_seen > 0
           AND ((?1 - last_seen) > CASE
                WHEN health_timeout_secs > 0 THEN health_timeout_secs
                ELSE ?2
           END)",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map(params![now, threshold_secs], |row| row.get::<_, String>(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Retention helpers
// ---------------------------------------------------------------------------

/// Delete decisions older than the configured `retain_days` for each org.
pub fn apply_retention(conn: &Connection) {
    let mut stmt = match conn.prepare(
        "SELECT org, retain_days FROM retention",
    ) {
        Ok(s) => s,
        Err(_) => return,
    };

    let rows: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    for (org, retain_days) in rows {
        let cutoff = now - (retain_days * 86_400);
        let _ = conn.execute(
            "DELETE FROM decisions WHERE org = ?1 AND t < ?2",
            params![org, cutoff],
        );
        let _ = conn.execute(
            "DELETE FROM costs WHERE org = ?1 AND t < ?2",
            params![org, cutoff],
        );
    }
}
