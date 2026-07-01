//! REST API — reads from SQLite, accepts ingest from remote agents.

use super::db::{self, Db};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

// ── Auth helper ────────────────────────────────────────────────────────────

fn extract_org(db: &Db, headers: &HeaderMap) -> Result<String, (StatusCode, &'static str)> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = auth.strip_prefix("Bearer ").unwrap_or(auth);
    if token.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, "missing Authorization header"));
    }

    let conn = db.lock().unwrap();
    db::validate_key(&conn, token)
        .ok_or((StatusCode::FORBIDDEN, "invalid API key"))
}

// ── Ingest: POST /api/ingest/decision ──────────────────────────────────────

pub async fn ingest_decision(
    State(db): State<Db>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<IngestResponse>, (StatusCode, &'static str)> {
    let org = extract_org(&db, &headers)?;
    let conn = db.lock().unwrap();

    // Accept single object or array.
    let items = if body.is_array() {
        body.as_array().unwrap().clone()
    } else {
        vec![body]
    };

    let mut count = 0;
    for item in &items {
        db::insert_decision_from_json(&conn, item, &org);
        count += 1;
    }

    Ok(Json(IngestResponse { ingested: count }))
}

// ── Ingest: POST /api/ingest/cost ──────────────────────────────────────────

pub async fn ingest_cost(
    State(db): State<Db>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<IngestResponse>, (StatusCode, &'static str)> {
    let org = extract_org(&db, &headers)?;
    let conn = db.lock().unwrap();

    let items = if body.is_array() {
        body.as_array().unwrap().clone()
    } else {
        vec![body]
    };

    let mut count = 0;
    for item in &items {
        db::insert_cost_from_json(&conn, &item, &org);
        count += 1;
    }

    Ok(Json(IngestResponse { ingested: count }))
}

#[derive(Serialize)]
pub struct IngestResponse {
    ingested: usize,
}

// ── GET /api/overview ──────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Overview {
    total_decisions: i64,
    total_blocked: i64,
    total_allowed: i64,
    block_rate: f64,
    agent_count: i64,
    total_cost_usd: f64,
    total_tokens_in: i64,
    total_tokens_out: i64,
    safety_blocks: i64,
    values_blocks: i64,
}

pub async fn overview(State(db): State<Db>) -> Json<Overview> {
    let conn = db.lock().unwrap();

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
        .unwrap_or(0);
    let blocked: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions WHERE blocked = 1", [], |r| r.get(0))
        .unwrap_or(0);
    let agent_count: i64 = conn
        .query_row("SELECT COUNT(DISTINCT agent) FROM decisions", [], |r| r.get(0))
        .unwrap_or(0);

    // Count safety vs values blocks by scanning verdicts JSON.
    let mut safety_blocks: i64 = 0;
    let mut values_blocks: i64 = 0;
    {
        let mut stmt = conn
            .prepare("SELECT verdicts FROM decisions WHERE blocked = 1")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        for row in rows.flatten() {
            if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&row) {
                for v in &arr {
                    if v.get("refused").and_then(|x| x.as_bool()).unwrap_or(false) {
                        match v.get("rail").and_then(|x| x.as_str()) {
                            Some("safety") => safety_blocks += 1,
                            Some("values") => values_blocks += 1,
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let total_cost: f64 = conn
        .query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM costs", [], |r| r.get(0))
        .unwrap_or(0.0);
    let total_tokens_in: i64 = conn
        .query_row("SELECT COALESCE(SUM(tokens_in), 0) FROM costs", [], |r| r.get(0))
        .unwrap_or(0);
    let total_tokens_out: i64 = conn
        .query_row("SELECT COALESCE(SUM(tokens_out), 0) FROM costs", [], |r| r.get(0))
        .unwrap_or(0);

    let rate = if total > 0 { blocked as f64 / total as f64 * 100.0 } else { 0.0 };

    Json(Overview {
        total_decisions: total,
        total_blocked: blocked,
        total_allowed: total - blocked,
        block_rate: rate,
        agent_count,
        total_cost_usd: total_cost,
        total_tokens_in,
        total_tokens_out,
        safety_blocks,
        values_blocks,
    })
}

// ── GET /api/agents ────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AgentSummary {
    name: String,
    mode: String,
    total_decisions: i64,
    blocked: i64,
    allowed: i64,
    block_rate: f64,
    cost_usd: f64,
    tokens_in: i64,
    tokens_out: i64,
    first_seen: i64,
    last_seen: i64,
}

pub async fn agents(State(db): State<Db>) -> Json<Vec<AgentSummary>> {
    let conn = db.lock().unwrap();

    let mut stmt = conn
        .prepare(
            "SELECT
                agent,
                MAX(mode) as mode,
                COUNT(*) as total,
                SUM(blocked) as blocked,
                MIN(t) as first_seen,
                MAX(t) as last_seen
             FROM decisions
             GROUP BY agent
             ORDER BY last_seen DESC",
        )
        .unwrap();

    let agents: Vec<AgentSummary> = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let total: i64 = row.get(2)?;
            let blocked: i64 = row.get(3)?;
            Ok(AgentSummary {
                name: name.clone(),
                mode: row.get(1)?,
                total_decisions: total,
                blocked,
                allowed: total - blocked,
                block_rate: if total > 0 { blocked as f64 / total as f64 * 100.0 } else { 0.0 },
                cost_usd: 0.0,
                tokens_in: 0,
                tokens_out: 0,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // Attach cost data.
    let mut result = agents;
    for agent in &mut result {
        let costs = conn.query_row(
            "SELECT COALESCE(SUM(tokens_in),0), COALESCE(SUM(tokens_out),0), COALESCE(SUM(cost_usd),0)
             FROM costs WHERE agent = ?1",
            rusqlite::params![agent.name],
            |r| Ok((r.get::<_,i64>(0)?, r.get::<_,i64>(1)?, r.get::<_,f64>(2)?)),
        );
        if let Ok((ti, to, cost)) = costs {
            agent.tokens_in = ti;
            agent.tokens_out = to;
            agent.cost_usd = cost;
        }
    }

    Json(result)
}

// ── GET /api/decisions ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DecisionQuery {
    agent: Option<String>,
    blocked: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Serialize)]
pub struct DecisionEntry {
    id: i64,
    t: i64,
    agent: String,
    action: String,
    target: String,
    decision: String,
    blocked: bool,
    reason: String,
    mode: String,
    provenance_id: String,
    verdicts: serde_json::Value,
    provenance: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct DecisionPage {
    total: i64,
    offset: i64,
    limit: i64,
    decisions: Vec<DecisionEntry>,
}

pub async fn decisions(
    State(db): State<Db>,
    Query(q): Query<DecisionQuery>,
) -> Json<DecisionPage> {
    let conn = db.lock().unwrap();
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    // Build dynamic query.
    let mut where_clauses: Vec<String> = Vec::new();
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref agent) = q.agent {
        where_clauses.push(format!("agent = ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(agent.clone()));
    }
    if let Some(blocked) = q.blocked {
        where_clauses.push(format!("blocked = ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(blocked as i32));
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    // Count total.
    let count_sql = format!("SELECT COUNT(*) FROM decisions {where_sql}");
    let total: i64 = {
        let mut stmt = conn.prepare(&count_sql).unwrap();
        let params: Vec<&dyn rusqlite::types::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();
        stmt.query_row(params.as_slice(), |r| r.get(0)).unwrap_or(0)
    };

    // Fetch page (newest first).
    let select_sql = format!(
        "SELECT id, t, agent, action, target, decision, blocked, reason, mode, provenance_id, verdicts, provenance
         FROM decisions {where_sql}
         ORDER BY t DESC, id DESC
         LIMIT ?{} OFFSET ?{}",
        bind_values.len() + 1,
        bind_values.len() + 2,
    );
    bind_values.push(Box::new(limit));
    bind_values.push(Box::new(offset));

    let mut stmt = conn.prepare(&select_sql).unwrap();
    let params: Vec<&dyn rusqlite::types::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();

    let decisions: Vec<DecisionEntry> = stmt
        .query_map(params.as_slice(), |row| {
            let verdicts_str: String = row.get(10)?;
            let provenance_str: Option<String> = row.get(11)?;
            Ok(DecisionEntry {
                id: row.get(0)?,
                t: row.get(1)?,
                agent: row.get(2)?,
                action: row.get(3)?,
                target: row.get(4)?,
                decision: row.get(5)?,
                blocked: row.get::<_, i32>(6)? != 0,
                reason: row.get(7)?,
                mode: row.get(8)?,
                provenance_id: row.get(9)?,
                verdicts: serde_json::from_str(&verdicts_str).unwrap_or(serde_json::json!([])),
                provenance: provenance_str.and_then(|s| serde_json::from_str(&s).ok()),
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    Json(DecisionPage {
        total,
        offset,
        limit,
        decisions,
    })
}

// ── GET /api/report ────────────────────────────────────────────────────────

pub async fn report_csv(State(db): State<Db>) -> impl IntoResponse {
    let conn = db.lock().unwrap();

    let mut csv = String::from(
        "timestamp,agent,action,target,decision,blocked,reason,mode,provenance_id,safety_verdict,values_verdict\n",
    );

    let mut stmt = conn
        .prepare("SELECT t, agent, action, target, decision, blocked, reason, mode, provenance_id, verdicts FROM decisions ORDER BY t ASC")
        .unwrap();

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i32>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .unwrap();

    for row in rows.flatten() {
        let (t, agent, action, target, decision, blocked, reason, mode, pid, verdicts_str) = row;

        let ts = chrono::DateTime::from_timestamp(t / 1000, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_default();

        let mut safety = "clean";
        let mut values = "clean";
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&verdicts_str) {
            for v in &arr {
                if v.get("refused").and_then(|x| x.as_bool()).unwrap_or(false) {
                    match v.get("rail").and_then(|x| x.as_str()) {
                        Some("safety") => safety = "refused",
                        Some("values") => values = "refused",
                        _ => {}
                    }
                }
            }
        }

        let esc = |s: &str| -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };

        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            ts,
            esc(&agent),
            esc(&action),
            esc(&target),
            decision,
            blocked != 0,
            esc(&reason),
            mode,
            pid,
            safety,
            values
        ));
    }

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"glassbox-audit-report.csv\"",
            ),
        ],
        csv,
    )
}
