//! Glassbox Web — compliance dashboard + ingest API.
//!
//! `glassbox serve`    → launches the dashboard on :3120
//! `glassbox key new`  → generates an API key for agent ingest

pub mod api;
pub mod db;

use axum::{
    Router,
    response::Html,
    routing::{get, post},
};
use tower_http::cors::CorsLayer;
use std::net::SocketAddr;

const DASHBOARD_HTML: &str = include_str!("static/dashboard.html");

pub fn run(args: &[String]) -> i32 {
    let port: u16 = arg_val(args, "--port")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3120);

    let database = db::open();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let app = Router::new()
            // Dashboard.
            .route("/", get(serve_dashboard))
            // Read API.
            .route("/api/overview", get(api::overview))
            .route("/api/agents", get(api::agents))
            .route("/api/decisions", get(api::decisions))
            .route("/api/report", get(api::report_csv))
            // Ingest API (requires API key).
            .route("/api/ingest/decision", post(api::ingest_decision))
            .route("/api/ingest/cost", post(api::ingest_cost))
            .layer(CorsLayer::permissive())
            .with_state(database);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        eprintln!("  Glassbox Web Dashboard");
        eprintln!("  Dashboard  http://localhost:{port}");
        eprintln!("  Ingest     POST http://localhost:{port}/api/ingest/decision");
        eprintln!("  Report     GET  http://localhost:{port}/api/report");
        eprintln!();
        eprintln!("  Run `glassbox key new` to generate an API key for agent ingest.");
        eprintln!("  Press Ctrl-C to stop.\n");

        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });
    0
}

/// `glassbox key new [label]` — generate an API key.
/// `glassbox key list`        — show active keys.
pub fn cmd_key(args: &[String]) -> i32 {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("help");
    let database = db::open();
    let conn = database.lock().unwrap();

    match sub {
        "new" | "create" => {
            let label = args.get(3).map(|s| s.as_str()).unwrap_or("default");
            let org = args.get(4).map(|s| s.as_str()).unwrap_or("default");
            let key = db::create_api_key(&conn, label, org);
            eprintln!("  API key created. Store this — it won't be shown again.\n");
            println!("{key}");
            eprintln!();
            eprintln!("  Usage:");
            eprintln!("    curl -X POST http://localhost:3120/api/ingest/decision \\");
            eprintln!("      -H 'Authorization: Bearer {key}' \\");
            eprintln!("      -H 'Content-Type: application/json' \\");
            eprintln!("      -d '{{\"agent\":\"my-agent\",\"action\":\"git push\",\"blocked\":false,\"decision\":\"allow\",\"t\":0,\"verdicts\":[]}}'");
            0
        }
        "list" | "ls" => {
            let mut stmt = conn
                .prepare("SELECT label, org, created_at, active FROM api_keys ORDER BY created_at DESC")
                .unwrap();
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i32>(3)?,
                    ))
                })
                .unwrap();

            eprintln!("  API Keys:\n");
            let mut count = 0;
            for row in rows.flatten() {
                let (label, org, created, active) = row;
                let status = if active == 1 { "active" } else { "revoked" };
                eprintln!("    {label}  org={org}  {status}  created={created}");
                count += 1;
            }
            if count == 0 {
                eprintln!("    (none)  Run `glassbox key new` to create one.");
            }
            0
        }
        _ => {
            eprintln!("usage: glassbox key [new [label] [org] | list]");
            2
        }
    }
}

async fn serve_dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

fn arg_val<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
}
