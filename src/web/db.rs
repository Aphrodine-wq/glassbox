//! SQLite backend for Glassbox.
//!
//! Tables:
//!   decisions  — every governed action, the core audit trail
//!   costs      — token/spend events per agent
//!   api_keys   — bearer tokens for ingest auth
//!
//! On first run, migrates the schema and imports any existing JSONL data.

use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type Db = Arc<Mutex<Connection>>;

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
        ",
    )
    .expect("migration failed");
}

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
}

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
