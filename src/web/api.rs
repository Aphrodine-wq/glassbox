//! REST API — full feature set for governance, cost control, compliance.

use super::db::{self, Db};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

// ── Auth ───────────────────────────────────────────────────────────────────

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
    db::validate_key(&conn, token).ok_or((StatusCode::FORBIDDEN, "invalid API key"))
}

// ── Ingest: POST /api/ingest/decision ──────────────────────────────────────

#[derive(Serialize)]
pub struct IngestResponse {
    ingested: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    killed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kill_reason: Option<String>,
}

pub async fn ingest_decision(
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
    let mut killed = false;
    let mut kill_reason = String::new();

    for item in &items {
        // Check if agent is killed.
        let agent = item.get("agent").and_then(|x| x.as_str()).unwrap_or("unknown");
        if let Ok(k) = conn.query_row(
            "SELECT killed, kill_reason FROM agent_config WHERE agent = ?1",
            rusqlite::params![agent],
            |r| Ok((r.get::<_, i32>(0)?, r.get::<_, String>(1)?)),
        ) {
            if k.0 != 0 {
                killed = true;
                kill_reason = k.1;
                continue; // skip ingesting for killed agents
            }
        }

        db::insert_decision_from_json(&conn, item, &org);
        count += 1;
    }

    Ok(Json(IngestResponse {
        ingested: count,
        killed: if killed { Some(true) } else { None },
        kill_reason: if killed { Some(kill_reason) } else { None },
    }))
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
    Ok(Json(IngestResponse { ingested: count, killed: None, kill_reason: None }))
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
    silent_agents: Vec<String>,
    budget_alerts: Vec<BudgetAlert>,
}

#[derive(Serialize)]
pub struct BudgetAlert {
    agent: String,
    max_usd: f64,
    spent_usd: f64,
    pct_used: f64,
    exceeded: bool,
}

pub async fn overview(State(db): State<Db>) -> Json<Overview> {
    let conn = db.lock().unwrap();

    let total: i64 = conn.query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0)).unwrap_or(0);
    let blocked: i64 = conn.query_row("SELECT COUNT(*) FROM decisions WHERE blocked = 1", [], |r| r.get(0)).unwrap_or(0);
    let agent_count: i64 = conn.query_row("SELECT COUNT(DISTINCT agent) FROM decisions", [], |r| r.get(0)).unwrap_or(0);

    let mut safety_blocks: i64 = 0;
    let mut values_blocks: i64 = 0;
    {
        let mut stmt = conn.prepare("SELECT verdicts FROM decisions WHERE blocked = 1").unwrap();
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

    let total_cost: f64 = conn.query_row("SELECT COALESCE(SUM(cost_usd), 0) FROM costs", [], |r| r.get(0)).unwrap_or(0.0);
    let total_tokens_in: i64 = conn.query_row("SELECT COALESCE(SUM(tokens_in), 0) FROM costs", [], |r| r.get(0)).unwrap_or(0);
    let total_tokens_out: i64 = conn.query_row("SELECT COALESCE(SUM(tokens_out), 0) FROM costs", [], |r| r.get(0)).unwrap_or(0);

    let silent_agents = db::get_silent_agents(&conn, 300);
    let budget_alerts = get_all_budget_alerts(&conn);

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
        silent_agents,
        budget_alerts,
    })
}

fn get_all_budget_alerts(conn: &rusqlite::Connection) -> Vec<BudgetAlert> {
    let mut alerts = Vec::new();
    let mut stmt = match conn.prepare("SELECT agent, max_usd, period, alert_at_pct FROM budgets") {
        Ok(s) => s,
        Err(_) => return alerts,
    };
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_,String>(0)?, r.get::<_,f64>(1)?, r.get::<_,String>(2)?, r.get::<_,f64>(3)?))
    });
    for row in rows.into_iter().flatten().flatten() {
        let (agent, max_usd, period, alert_pct) = row;
        let spent = period_spend(conn, &agent, &period);
        let pct = if max_usd > 0.0 { spent / max_usd * 100.0 } else { 0.0 };
        if pct >= alert_pct {
            alerts.push(BudgetAlert { agent, max_usd, spent_usd: spent, pct_used: pct, exceeded: pct >= 100.0 });
        }
    }
    alerts
}

fn period_spend(conn: &rusqlite::Connection, agent: &str, period: &str) -> f64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let cutoff = match period {
        "daily" => now - 86_400_000,
        "weekly" => now - 604_800_000,
        _ => now - 2_592_000_000, // monthly
    };
    conn.query_row(
        "SELECT COALESCE(SUM(cost_usd), 0) FROM costs WHERE agent = ?1 AND t >= ?2",
        rusqlite::params![agent, cutoff],
        |r| r.get(0),
    ).unwrap_or(0.0)
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
    killed: bool,
    group_name: String,
    team: String,
    environment: String,
    health: String,
}

pub async fn agents(State(db): State<Db>) -> Json<Vec<AgentSummary>> {
    let conn = db.lock().unwrap();

    let mut stmt = conn.prepare(
        "SELECT agent, MAX(mode), COUNT(*), SUM(blocked), MIN(t), MAX(t)
         FROM decisions GROUP BY agent ORDER BY MAX(t) DESC",
    ).unwrap();

    let mut agents: Vec<AgentSummary> = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let total: i64 = row.get(2)?;
            let blocked: i64 = row.get(3)?;
            Ok(AgentSummary {
                name, mode: row.get(1)?, total_decisions: total, blocked,
                allowed: total - blocked,
                block_rate: if total > 0 { blocked as f64 / total as f64 * 100.0 } else { 0.0 },
                cost_usd: 0.0, tokens_in: 0, tokens_out: 0,
                first_seen: row.get(4)?, last_seen: row.get(5)?,
                killed: false, group_name: String::new(), team: String::new(),
                environment: String::new(), health: "ok".into(),
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    for agent in &mut agents {
        // Costs.
        if let Ok((ti, to, cost)) = conn.query_row(
            "SELECT COALESCE(SUM(tokens_in),0), COALESCE(SUM(tokens_out),0), COALESCE(SUM(cost_usd),0)
             FROM costs WHERE agent = ?1",
            rusqlite::params![agent.name],
            |r| Ok((r.get::<_,i64>(0)?, r.get::<_,i64>(1)?, r.get::<_,f64>(2)?)),
        ) {
            agent.tokens_in = ti;
            agent.tokens_out = to;
            agent.cost_usd = cost;
        }
        // Config.
        if let Ok((killed, group, team, env, last_seen, timeout)) = conn.query_row(
            "SELECT killed, group_name, team, environment, last_seen, health_timeout_secs FROM agent_config WHERE agent = ?1",
            rusqlite::params![agent.name],
            |r| Ok((r.get::<_,i32>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?,
                     r.get::<_,String>(3)?, r.get::<_,i64>(4)?, r.get::<_,i64>(5)?)),
        ) {
            agent.killed = killed != 0;
            agent.group_name = group;
            agent.team = team;
            agent.environment = env;
            let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            let last_s = last_seen / 1000;
            if last_s > 0 && (now_s - last_s) > timeout {
                agent.health = "silent".into();
            }
        }
    }

    Json(agents)
}

// ── GET /api/agents/:name/timeline ─────────────────────────────────────────

#[derive(Serialize)]
pub struct TimelineEntry {
    t: i64,
    action: String,
    decision: String,
    blocked: bool,
    reason: String,
}

pub async fn agent_timeline(
    State(db): State<Db>,
    Path(name): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Json<Vec<TimelineEntry>> {
    let conn = db.lock().unwrap();
    let limit = q.limit.unwrap_or(100).min(500);
    let mut stmt = conn.prepare(
        "SELECT t, action, decision, blocked, reason FROM decisions WHERE agent = ?1 ORDER BY t DESC LIMIT ?2"
    ).unwrap();
    let entries: Vec<TimelineEntry> = stmt.query_map(
        rusqlite::params![name, limit],
        |r| Ok(TimelineEntry {
            t: r.get(0)?, action: r.get(1)?, decision: r.get(2)?,
            blocked: r.get::<_,i32>(3)? != 0, reason: r.get(4)?,
        }),
    ).unwrap().filter_map(|r| r.ok()).collect();
    Json(entries)
}

#[derive(Deserialize)]
pub struct LimitQuery {
    limit: Option<i64>,
}

// ── POST /api/agents/:name/kill ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct KillRequest {
    reason: Option<String>,
}

pub async fn agent_kill(
    State(db): State<Db>,
    Path(name): Path<String>,
    Json(body): Json<KillRequest>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    let reason = body.reason.unwrap_or_else(|| "manually killed via API".into());
    conn.execute(
        "INSERT INTO agent_config (agent, killed, kill_reason) VALUES (?1, 1, ?2)
         ON CONFLICT(agent) DO UPDATE SET killed = 1, kill_reason = ?2",
        rusqlite::params![name, reason],
    ).unwrap();
    Json(serde_json::json!({"agent": name, "killed": true, "reason": reason}))
}

// ── POST /api/agents/:name/revive ──────────────────────────────────────────

pub async fn agent_revive(
    State(db): State<Db>,
    Path(name): Path<String>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE agent_config SET killed = 0, kill_reason = '' WHERE agent = ?1",
        rusqlite::params![name],
    ).unwrap();
    Json(serde_json::json!({"agent": name, "killed": false}))
}

// ── POST /api/agents/:name/mode ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ModeRequest {
    mode: String,
}

pub async fn agent_mode(
    State(db): State<Db>,
    Path(name): Path<String>,
    Json(body): Json<ModeRequest>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    let mode = if body.mode == "enforce" { "enforce" } else { "shadow" };
    conn.execute(
        "INSERT INTO agent_config (agent, mode) VALUES (?1, ?2)
         ON CONFLICT(agent) DO UPDATE SET mode = ?2",
        rusqlite::params![name, mode],
    ).unwrap();
    Json(serde_json::json!({"agent": name, "mode": mode}))
}

// ── GET /api/decisions ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DecisionQuery {
    agent: Option<String>,
    blocked: Option<bool>,
    team: Option<String>,
    environment: Option<String>,
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

pub async fn decisions(State(db): State<Db>, Query(q): Query<DecisionQuery>) -> Json<DecisionPage> {
    let conn = db.lock().unwrap();
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let mut wheres: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref agent) = q.agent {
        wheres.push(format!("d.agent = ?{}", vals.len() + 1));
        vals.push(Box::new(agent.clone()));
    }
    if let Some(blocked) = q.blocked {
        wheres.push(format!("d.blocked = ?{}", vals.len() + 1));
        vals.push(Box::new(blocked as i32));
    }
    if let Some(ref team) = q.team {
        wheres.push(format!("ac.team = ?{}", vals.len() + 1));
        vals.push(Box::new(team.clone()));
    }
    if let Some(ref env) = q.environment {
        wheres.push(format!("ac.environment = ?{}", vals.len() + 1));
        vals.push(Box::new(env.clone()));
    }

    let join = if q.team.is_some() || q.environment.is_some() {
        "LEFT JOIN agent_config ac ON d.agent = ac.agent"
    } else {
        ""
    };

    let where_sql = if wheres.is_empty() { String::new() } else { format!("WHERE {}", wheres.join(" AND ")) };

    let count_sql = format!("SELECT COUNT(*) FROM decisions d {join} {where_sql}");
    let total: i64 = {
        let mut stmt = conn.prepare(&count_sql).unwrap();
        let params: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
        stmt.query_row(params.as_slice(), |r| r.get(0)).unwrap_or(0)
    };

    let select_sql = format!(
        "SELECT d.id, d.t, d.agent, d.action, d.target, d.decision, d.blocked, d.reason, d.mode, d.provenance_id, d.verdicts, d.provenance
         FROM decisions d {join} {where_sql}
         ORDER BY d.t DESC, d.id DESC LIMIT ?{} OFFSET ?{}",
        vals.len() + 1, vals.len() + 2,
    );
    vals.push(Box::new(limit));
    vals.push(Box::new(offset));

    let mut stmt = conn.prepare(&select_sql).unwrap();
    let params: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();

    let decisions: Vec<DecisionEntry> = stmt.query_map(params.as_slice(), |row| {
        let vs: String = row.get(10)?;
        let ps: Option<String> = row.get(11)?;
        Ok(DecisionEntry {
            id: row.get(0)?, t: row.get(1)?, agent: row.get(2)?, action: row.get(3)?,
            target: row.get(4)?, decision: row.get(5)?, blocked: row.get::<_,i32>(6)? != 0,
            reason: row.get(7)?, mode: row.get(8)?, provenance_id: row.get(9)?,
            verdicts: serde_json::from_str(&vs).unwrap_or(serde_json::json!([])),
            provenance: ps.and_then(|s| serde_json::from_str(&s).ok()),
        })
    }).unwrap().filter_map(|r| r.ok()).collect();

    Json(DecisionPage { total, offset, limit, decisions })
}

// ── GET /api/search ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    limit: Option<i64>,
}

pub async fn search(State(db): State<Db>, Query(sq): Query<SearchQuery>) -> Json<DecisionPage> {
    let conn = db.lock().unwrap();
    let limit = sq.limit.unwrap_or(50).min(200);
    let pattern = format!("%{}%", sq.q);

    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM decisions WHERE action LIKE ?1 OR reason LIKE ?1 OR agent LIKE ?1 OR target LIKE ?1",
        rusqlite::params![pattern], |r| r.get(0),
    ).unwrap_or(0);

    let mut stmt = conn.prepare(
        "SELECT id, t, agent, action, target, decision, blocked, reason, mode, provenance_id, verdicts, provenance
         FROM decisions WHERE action LIKE ?1 OR reason LIKE ?1 OR agent LIKE ?1 OR target LIKE ?1
         ORDER BY t DESC LIMIT ?2"
    ).unwrap();

    let decisions: Vec<DecisionEntry> = stmt.query_map(
        rusqlite::params![pattern, limit],
        |row| {
            let vs: String = row.get(10)?;
            let ps: Option<String> = row.get(11)?;
            Ok(DecisionEntry {
                id: row.get(0)?, t: row.get(1)?, agent: row.get(2)?, action: row.get(3)?,
                target: row.get(4)?, decision: row.get(5)?, blocked: row.get::<_,i32>(6)? != 0,
                reason: row.get(7)?, mode: row.get(8)?, provenance_id: row.get(9)?,
                verdicts: serde_json::from_str(&vs).unwrap_or(serde_json::json!([])),
                provenance: ps.and_then(|s| serde_json::from_str(&s).ok()),
            })
        },
    ).unwrap().filter_map(|r| r.ok()).collect();

    Json(DecisionPage { total, offset: 0, limit, decisions })
}

// ── Budget CRUD: /api/budgets ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct BudgetCreate {
    agent: String,
    max_usd: f64,
    period: Option<String>,
    alert_at_pct: Option<f64>,
    kill_on_exceed: Option<bool>,
}

#[derive(Serialize)]
pub struct BudgetInfo {
    id: i64,
    agent: String,
    max_usd: f64,
    spent_usd: f64,
    pct_used: f64,
    period: String,
    alert_at_pct: f64,
    kill_on_exceed: bool,
    exceeded: bool,
}

pub async fn budget_create(
    State(db): State<Db>,
    Json(body): Json<BudgetCreate>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    let period = body.period.unwrap_or_else(|| "monthly".into());
    let alert = body.alert_at_pct.unwrap_or(80.0);
    let kill = body.kill_on_exceed.unwrap_or(false) as i32;
    conn.execute(
        "INSERT INTO budgets (agent, max_usd, period, alert_at_pct, kill_on_exceed) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![body.agent, body.max_usd, period, alert, kill],
    ).unwrap();
    Json(serde_json::json!({"created": true, "agent": body.agent, "max_usd": body.max_usd}))
}

pub async fn budget_list(State(db): State<Db>) -> Json<Vec<BudgetInfo>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, agent, max_usd, period, alert_at_pct, kill_on_exceed FROM budgets"
    ).unwrap();
    let budgets: Vec<BudgetInfo> = stmt.query_map([], |r| {
        let agent: String = r.get(1)?;
        let max_usd: f64 = r.get(2)?;
        let period: String = r.get(3)?;
        let spent = period_spend(&conn, &agent, &period);
        let pct = if max_usd > 0.0 { spent / max_usd * 100.0 } else { 0.0 };
        Ok(BudgetInfo {
            id: r.get(0)?, agent, max_usd, spent_usd: spent, pct_used: pct,
            period, alert_at_pct: r.get(4)?,
            kill_on_exceed: r.get::<_,i32>(5)? != 0, exceeded: pct >= 100.0,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(budgets)
}

pub async fn budget_delete(State(db): State<Db>, Path(id): Path<i64>) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM budgets WHERE id = ?1", rusqlite::params![id]).unwrap();
    Json(serde_json::json!({"deleted": true}))
}

// ── Webhook CRUD: /api/webhooks ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct WebhookCreate {
    url: String,
    events: Option<String>,
}

#[derive(Serialize)]
pub struct WebhookInfo {
    id: i64,
    url: String,
    events: String,
    active: bool,
}

pub async fn webhook_create(
    State(db): State<Db>,
    Json(body): Json<WebhookCreate>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    let events = body.events.unwrap_or_else(|| "block".into());
    conn.execute(
        "INSERT INTO webhooks (url, events) VALUES (?1, ?2)",
        rusqlite::params![body.url, events],
    ).unwrap();
    Json(serde_json::json!({"created": true, "url": body.url}))
}

pub async fn webhook_list(State(db): State<Db>) -> Json<Vec<WebhookInfo>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id, url, events, active FROM webhooks").unwrap();
    let hooks: Vec<WebhookInfo> = stmt.query_map([], |r| Ok(WebhookInfo {
        id: r.get(0)?, url: r.get(1)?, events: r.get(2)?, active: r.get::<_,i32>(3)? != 0,
    })).unwrap().filter_map(|r| r.ok()).collect();
    Json(hooks)
}

pub async fn webhook_delete(State(db): State<Db>, Path(id): Path<i64>) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM webhooks WHERE id = ?1", rusqlite::params![id]).unwrap();
    Json(serde_json::json!({"deleted": true}))
}

// ── Policy CRUD: /api/policies ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PolicyCreate {
    name: String,
    description: Option<String>,
    template: Option<String>,
    rules: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct PolicyInfo {
    id: i64,
    name: String,
    description: String,
    template: String,
    rules: serde_json::Value,
    active: bool,
}

pub async fn policy_create(
    State(db): State<Db>,
    Json(body): Json<PolicyCreate>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    let desc = body.description.unwrap_or_default();
    let tmpl = body.template.unwrap_or_default();
    let rules = body.rules.unwrap_or(serde_json::json!([])).to_string();
    conn.execute(
        "INSERT INTO policies (name, description, template, rules) VALUES (?1,?2,?3,?4)",
        rusqlite::params![body.name, desc, tmpl, rules],
    ).unwrap();
    Json(serde_json::json!({"created": true, "name": body.name}))
}

pub async fn policy_list(State(db): State<Db>) -> Json<Vec<PolicyInfo>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id, name, description, template, rules, active FROM policies").unwrap();
    let policies: Vec<PolicyInfo> = stmt.query_map([], |r| {
        let rules_str: String = r.get(4)?;
        Ok(PolicyInfo {
            id: r.get(0)?, name: r.get(1)?, description: r.get(2)?, template: r.get(3)?,
            rules: serde_json::from_str(&rules_str).unwrap_or(serde_json::json!([])),
            active: r.get::<_,i32>(5)? != 0,
        })
    }).unwrap().filter_map(|r| r.ok()).collect();
    Json(policies)
}

pub async fn policy_delete(State(db): State<Db>, Path(id): Path<i64>) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM policies WHERE id = ?1", rusqlite::params![id]).unwrap();
    Json(serde_json::json!({"deleted": true}))
}

// ── Seed default policy templates ──────────────────────────────────────────

pub fn seed_policies(db: &Db) {
    let conn = db.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM policies", [], |r| r.get(0)).unwrap_or(0);
    if count > 0 { return; }

    let templates = [
        ("SOC2 AI Agent Controls", "soc2", "Controls for SOC2 compliance when deploying AI agents", serde_json::json!([
            {"rule": "block_irreversible", "description": "Block all irreversible operations (rm -rf, DROP TABLE, force push)", "rail": "safety", "severity": "critical"},
            {"rule": "enforce_mode_required", "description": "All production agents must run in enforce mode", "rail": "safety", "severity": "high"},
            {"rule": "audit_trail_required", "description": "Every agent action must be logged with provenance", "rail": "compliance", "severity": "critical"},
            {"rule": "cost_budget_required", "description": "All agents must have a spend budget configured", "rail": "cost", "severity": "medium"},
            {"rule": "human_escalation", "description": "Blocked actions must be escalatable to a human", "rail": "safety", "severity": "high"},
        ])),
        ("HIPAA AI Agent Controls", "hipaa", "Controls for HIPAA compliance in healthcare AI deployments", serde_json::json!([
            {"rule": "no_phi_in_logs", "description": "Agent actions must not contain PHI in the action string", "rail": "values", "severity": "critical"},
            {"rule": "block_external_transfer", "description": "Block agent actions that transfer data to external services", "rail": "safety", "severity": "critical"},
            {"rule": "encrypt_at_rest", "description": "Audit logs must be encrypted at rest", "rail": "compliance", "severity": "critical"},
            {"rule": "access_logging", "description": "All access to patient data must be logged", "rail": "compliance", "severity": "critical"},
            {"rule": "minimum_necessary", "description": "Agents should only access minimum necessary data", "rail": "values", "severity": "high"},
        ])),
        ("Internal Development", "internal", "Standard controls for internal development AI agents", serde_json::json!([
            {"rule": "block_production_access", "description": "Dev agents cannot access production databases or APIs", "rail": "safety", "severity": "high"},
            {"rule": "shadow_first", "description": "New agents must run in shadow mode for 7 days before enforce", "rail": "safety", "severity": "medium"},
            {"rule": "cost_alert", "description": "Alert when daily spend exceeds $50", "rail": "cost", "severity": "medium"},
            {"rule": "no_secrets_in_commits", "description": "Block commits containing API keys or secrets", "rail": "safety", "severity": "critical"},
        ])),
    ];

    for (name, tmpl, desc, rules) in templates {
        let _ = conn.execute(
            "INSERT INTO policies (name, description, template, rules) VALUES (?1,?2,?3,?4)",
            rusqlite::params![name, desc, tmpl, rules.to_string()],
        );
    }
}

// ── Cost forecast: GET /api/forecast ───────────────────────────────────────

#[derive(Serialize)]
pub struct Forecast {
    agent: String,
    daily_avg: f64,
    projected_monthly: f64,
    model_breakdown: Vec<ModelCost>,
}

#[derive(Serialize)]
pub struct ModelCost {
    model: String,
    cost_usd: f64,
    tokens: i64,
}

pub async fn forecast(State(db): State<Db>) -> Json<Vec<Forecast>> {
    let conn = db.lock().unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let week_ago = now - 604_800_000;

    let mut stmt = conn.prepare(
        "SELECT agent, SUM(cost_usd), SUM(tokens_in + tokens_out)
         FROM costs WHERE t >= ?1 GROUP BY agent"
    ).unwrap();

    let mut forecasts: Vec<Forecast> = stmt.query_map(
        rusqlite::params![week_ago],
        |r| {
            let agent: String = r.get(0)?;
            let week_cost: f64 = r.get(1)?;
            let daily = week_cost / 7.0;
            Ok(Forecast {
                agent, daily_avg: daily, projected_monthly: daily * 30.0,
                model_breakdown: Vec::new(),
            })
        },
    ).unwrap().filter_map(|r| r.ok()).collect();

    // Model breakdown per agent.
    for f in &mut forecasts {
        let mut mstmt = conn.prepare(
            "SELECT COALESCE(NULLIF(model,''),'unknown'), SUM(cost_usd), SUM(tokens_in + tokens_out)
             FROM costs WHERE agent = ?1 GROUP BY COALESCE(NULLIF(model,''),'unknown')"
        ).unwrap();
        f.model_breakdown = mstmt.query_map(
            rusqlite::params![f.agent],
            |r| Ok(ModelCost { model: r.get(0)?, cost_usd: r.get(1)?, tokens: r.get(2)? }),
        ).unwrap().filter_map(|r| r.ok()).collect();
    }

    Json(forecasts)
}

// ── Agent groups: /api/groups ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct GroupUpdate {
    agent: String,
    group_name: Option<String>,
    team: Option<String>,
    environment: Option<String>,
}

pub async fn agent_group_update(
    State(db): State<Db>,
    Json(body): Json<GroupUpdate>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    let group = body.group_name.unwrap_or_default();
    let team = body.team.unwrap_or_default();
    let env = body.environment.unwrap_or_else(|| "development".into());
    conn.execute(
        "INSERT INTO agent_config (agent, group_name, team, environment) VALUES (?1,?2,?3,?4)
         ON CONFLICT(agent) DO UPDATE SET group_name=?2, team=?3, environment=?4",
        rusqlite::params![body.agent, group, team, env],
    ).unwrap();
    Json(serde_json::json!({"updated": true, "agent": body.agent}))
}

// ── Retention: /api/retention ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RetentionConfig {
    retain_days: i64,
}

pub async fn retention_set(
    State(db): State<Db>,
    Json(body): Json<RetentionConfig>,
) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    conn.execute("DELETE FROM retention WHERE org = 'default'", []).unwrap();
    conn.execute(
        "INSERT INTO retention (org, retain_days) VALUES ('default', ?1)",
        rusqlite::params![body.retain_days],
    ).unwrap();
    Json(serde_json::json!({"retain_days": body.retain_days}))
}

pub async fn retention_apply(State(db): State<Db>) -> Json<serde_json::Value> {
    let conn = db.lock().unwrap();
    db::apply_retention(&conn);
    Json(serde_json::json!({"applied": true}))
}

// ── Audit diff: GET /api/audit/diff ────────────────────────────────────────

#[derive(Deserialize)]
pub struct DiffQuery {
    from: i64, // epoch ms
    to: i64,
}

#[derive(Serialize)]
pub struct AuditDiff {
    period_from: i64,
    period_to: i64,
    decisions_added: i64,
    blocks_added: i64,
    new_agents: Vec<String>,
    cost_delta: f64,
    block_rate_from: f64,
    block_rate_to: f64,
}

pub async fn audit_diff(State(db): State<Db>, Query(q): Query<DiffQuery>) -> Json<AuditDiff> {
    let conn = db.lock().unwrap();
    let duration = q.to - q.from;
    let prev_from = q.from - duration;
    let prev_to = q.from;

    let count_in = |from: i64, to: i64| -> (i64, i64) {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM decisions WHERE t >= ?1 AND t < ?2",
            rusqlite::params![from, to], |r| r.get(0),
        ).unwrap_or(0);
        let blocked: i64 = conn.query_row(
            "SELECT COUNT(*) FROM decisions WHERE t >= ?1 AND t < ?2 AND blocked = 1",
            rusqlite::params![from, to], |r| r.get(0),
        ).unwrap_or(0);
        (total, blocked)
    };

    let (prev_total, prev_blocked) = count_in(prev_from, prev_to);
    let (curr_total, curr_blocked) = count_in(q.from, q.to);

    let prev_cost: f64 = conn.query_row(
        "SELECT COALESCE(SUM(cost_usd),0) FROM costs WHERE t >= ?1 AND t < ?2",
        rusqlite::params![prev_from, prev_to], |r| r.get(0),
    ).unwrap_or(0.0);
    let curr_cost: f64 = conn.query_row(
        "SELECT COALESCE(SUM(cost_usd),0) FROM costs WHERE t >= ?1 AND t < ?2",
        rusqlite::params![q.from, q.to], |r| r.get(0),
    ).unwrap_or(0.0);

    // Agents in current period not in previous.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT agent FROM decisions WHERE t >= ?1 AND t < ?2
         AND agent NOT IN (SELECT DISTINCT agent FROM decisions WHERE t >= ?3 AND t < ?4)"
    ).unwrap();
    let new_agents: Vec<String> = stmt.query_map(
        rusqlite::params![q.from, q.to, prev_from, prev_to],
        |r| r.get(0),
    ).unwrap().filter_map(|r| r.ok()).collect();

    let prev_rate = if prev_total > 0 { prev_blocked as f64 / prev_total as f64 * 100.0 } else { 0.0 };
    let curr_rate = if curr_total > 0 { curr_blocked as f64 / curr_total as f64 * 100.0 } else { 0.0 };

    Json(AuditDiff {
        period_from: q.from,
        period_to: q.to,
        decisions_added: curr_total - prev_total,
        blocks_added: curr_blocked - prev_blocked,
        new_agents,
        cost_delta: curr_cost - prev_cost,
        block_rate_from: prev_rate,
        block_rate_to: curr_rate,
    })
}

// ── CSV report ─────────────────────────────────────────────────────────────

pub async fn report_csv(State(db): State<Db>) -> impl IntoResponse {
    let conn = db.lock().unwrap();
    let mut csv = String::from("timestamp,agent,action,target,decision,blocked,reason,mode,provenance_id,safety_verdict,values_verdict\n");

    let mut stmt = conn.prepare(
        "SELECT t, agent, action, target, decision, blocked, reason, mode, provenance_id, verdicts FROM decisions ORDER BY t ASC"
    ).unwrap();

    let rows = stmt.query_map([], |row| Ok((
        row.get::<_,i64>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,
        row.get::<_,String>(3)?, row.get::<_,String>(4)?, row.get::<_,i32>(5)?,
        row.get::<_,String>(6)?, row.get::<_,String>(7)?, row.get::<_,String>(8)?,
        row.get::<_,String>(9)?,
    ))).unwrap();

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
            } else { s.to_string() }
        };

        csv.push_str(&format!("{},{},{},{},{},{},{},{},{},{},{}\n",
            ts, esc(&agent), esc(&action), esc(&target), decision, blocked != 0,
            esc(&reason), mode, pid, safety, values));
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/csv"),
        (header::CONTENT_DISPOSITION, "attachment; filename=\"glassbox-audit-report.csv\"")], csv)
}
