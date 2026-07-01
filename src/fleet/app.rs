//! App state — agents, decisions, and cost tracking derived from the decision log.

use std::collections::HashMap;

/// Per-agent state derived from the decision stream.
pub struct AgentInfo {
    pub name: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub total_decisions: u64,
    pub blocked_count: u64,
    pub would_block_count: u64,
    pub mode: String,
    /// Estimated token cost (placeholder — will be populated from cost events).
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    /// Recent activity: last 20 decisions as (blocked: bool) for sparkline.
    pub activity: Vec<bool>,
}

/// A single decision from the log, kept lightweight for the TUI.
pub struct Decision {
    pub t: u64,
    pub agent: String,
    pub action: String,
    pub target: String,
    pub decision: String,
    pub blocked: bool,
    pub reason: String,
    pub mode: String,
    pub rails: Vec<(String, bool)>, // (rail_name, refused)
}

pub struct App {
    /// Path to the decisions log.
    pub log_path: String,
    /// Last known file length — skip reload if unchanged.
    last_len: u64,

    /// Agent registry, keyed by agent name.
    pub agents: Vec<AgentInfo>,
    /// All decisions (most recent last).
    pub decisions: Vec<Decision>,

    /// Currently selected agent index in the left pane.
    pub selected_agent: usize,
    /// Scroll offset for the center pane.
    pub decision_scroll: usize,

    /// Fleet-wide totals.
    pub total_decisions: u64,
    pub total_blocked: u64,
    pub total_cost_usd: f64,

    /// Active pane: 0=agents, 1=decisions, 2=cost.
    pub active_pane: u8,
    /// Show all agents' decisions vs filtered.
    pub show_all: bool,
}

impl App {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        App {
            log_path: format!("{home}/.glassbox/decisions.jsonl"),
            last_len: u64::MAX, // force first load
            agents: Vec::new(),
            decisions: Vec::new(),
            selected_agent: 0,
            decision_scroll: 0,
            total_decisions: 0,
            total_blocked: 0,
            total_cost_usd: 0.0,
            active_pane: 0,
            show_all: false,
        }
    }

    /// Reload from the JSONL log if the file has changed.
    pub fn reload(&mut self) {
        let len = std::fs::metadata(&self.log_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if len == self.last_len {
            return;
        }
        self.last_len = len;

        let text = match std::fs::read_to_string(&self.log_path) {
            Ok(t) => t,
            Err(_) => return,
        };

        let mut agent_map: HashMap<String, AgentInfo> = HashMap::new();
        let mut decisions = Vec::new();

        for line in text.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let agent_name = v
                .get("agent")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            let t = v.get("t").and_then(|x| x.as_u64()).unwrap_or(0);
            let blocked = v.get("blocked").and_then(|x| x.as_bool()).unwrap_or(false);
            let decision_label = v
                .get("decision")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let mode = v
                .get("mode")
                .and_then(|x| x.as_str())
                .unwrap_or("shadow")
                .to_string();

            // Parse rails from verdicts array.
            let rails: Vec<(String, bool)> = v
                .get("verdicts")
                .and_then(|x| x.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|vj| {
                            let rail = vj
                                .get("rail")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let refused =
                                vj.get("refused").and_then(|x| x.as_bool()).unwrap_or(false);
                            (rail, refused)
                        })
                        .collect()
                })
                .unwrap_or_default();

            let dec = Decision {
                t,
                agent: agent_name.clone(),
                action: v
                    .get("action")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                target: v
                    .get("target")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                decision: decision_label.clone(),
                blocked,
                reason: v
                    .get("reason")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                mode: mode.clone(),
                rails,
            };
            decisions.push(dec);

            // Update agent registry.
            let info = agent_map.entry(agent_name.clone()).or_insert_with(|| AgentInfo {
                name: agent_name,
                first_seen: t,
                last_seen: t,
                total_decisions: 0,
                blocked_count: 0,
                would_block_count: 0,
                mode: mode.clone(),
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                activity: Vec::new(),
            });
            info.last_seen = t;
            info.total_decisions += 1;
            info.mode = mode;
            info.activity.push(blocked);
            if info.activity.len() > 20 {
                info.activity.remove(0);
            }
            if blocked {
                if decision_label == "would-refuse" {
                    info.would_block_count += 1;
                } else {
                    info.blocked_count += 1;
                }
            }
        }

        // Load cost events if they exist.
        self.load_costs(&mut agent_map);

        self.total_decisions = decisions.len() as u64;
        self.total_blocked = decisions.iter().filter(|d| d.blocked).count() as u64;
        self.total_cost_usd = agent_map.values().map(|a| a.cost_usd).sum();
        self.decisions = decisions;

        // Sort agents by last_seen descending (most active first).
        let mut agents: Vec<AgentInfo> = agent_map.into_values().collect();
        agents.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        self.agents = agents;

        // Clamp selection.
        if self.selected_agent >= self.agents.len() && !self.agents.is_empty() {
            self.selected_agent = self.agents.len() - 1;
        }
    }

    /// Load token/cost events from `~/.glassbox/costs.jsonl`.
    /// Format: `{"agent":"name","tokens_in":N,"tokens_out":N,"cost_usd":F,"t":N}`
    fn load_costs(&self, agent_map: &mut HashMap<String, AgentInfo>) {
        let cost_path = self.log_path.replace("decisions.jsonl", "costs.jsonl");
        let text = match std::fs::read_to_string(&cost_path) {
            Ok(t) => t,
            Err(_) => return,
        };
        for line in text.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let agent_name = v
                .get("agent")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            if let Some(info) = agent_map.get_mut(&agent_name) {
                info.tokens_in += v.get("tokens_in").and_then(|x| x.as_u64()).unwrap_or(0);
                info.tokens_out += v.get("tokens_out").and_then(|x| x.as_u64()).unwrap_or(0);
                info.cost_usd += v.get("cost_usd").and_then(|x| x.as_f64()).unwrap_or(0.0);
            }
        }
    }

    /// Decisions filtered to the currently selected agent, or all if show_all.
    pub fn filtered_decisions(&self) -> Vec<&Decision> {
        if self.agents.is_empty() || self.show_all {
            return self.decisions.iter().collect();
        }
        let agent = &self.agents[self.selected_agent].name;
        self.decisions
            .iter()
            .filter(|d| d.agent == *agent)
            .collect()
    }
}
