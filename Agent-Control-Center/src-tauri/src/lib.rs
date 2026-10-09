mod settings;

use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use walkdir::WalkDir;

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AgentState {
    team: String,
    role: String,
    thread_id: String,
    model: String,
    status: String,
    task: String,
    detail: String,
    current_file: String,
    command: String,
    started_at: String,
    updated_at: String,
    input_tokens: u64,
    output_tokens: u64,
    tokens_observed: bool,
    workspace: String,
    // Empty for direct sessions. Delegated states from the same parent are a
    // cohort, so a stale working sibling from another parent cannot win a card.
    parent_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Activity {
    id: i64,
    at: String,
    thread_id: String,
    role: String,
    kind: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityFilter {
    team: String,
    role: Option<String>,
    kind: Option<String>,
    query: Option<String>,
    before_id: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityPage {
    events: Vec<Activity>,
    has_more: bool,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ResearchMetrics {
    workspace: String,
    papers_found: u64,
    papers_screened: u64,
    papers_included: u64,
    datasets: u64,
    experiments_complete: u64,
    sections_complete: u64,
    sections_total: u64,
    citations_verified: u64,
    citations_unverified: u64,
    citations_broken: u64,
    review_status: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    connection: String,
    source: String,
    agents: Vec<AgentState>,
    activity: Vec<Activity>,
    research: ResearchMetrics,
    total_input: u64,
    total_output: u64,
    updated_at: u64,
}

struct Monitor {
    files: HashMap<PathBuf, u64>,
    ignored: HashSet<PathBuf>,
    agents: HashMap<String, AgentState>,
    session_aliases: HashMap<String, String>,
    list_agent_calls: HashSet<String>,
    seen_token_records: HashSet<String>,
    agent_token_records: HashSet<String>,
    observed_input_tokens: u64,
    observed_output_tokens: u64,
    activity: Vec<Activity>,
    db: Connection,
    next_id: i64,
    source_dir: PathBuf,
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn short(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}
fn roles_for_team(team: &str) -> Result<Vec<&'static str>, String> {
    match team {
        "software" => Ok(vec!["Planner", "Coder", "Mobile Dev", "Tester", "Reviewer"]),
        "research" => Ok(vec![
            "Research Lead",
            "Literature Researcher",
            "Data & Evidence",
            "Methodology Scientist",
            "Experiment Engineer",
            "Paper Writer",
            "Academic Reviewer",
        ]),
        _ => Err("Unknown activity team".into()),
    }
}
fn escape_like(query: &str) -> String {
    query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}
fn field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn normalized_item_type(item: &Value) -> String {
    field(item, "type")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn normalize_role(raw: &str) -> String {
    let s = raw.to_ascii_lowercase();
    if s.contains("literature") {
        "Literature Researcher"
    } else if s.contains("data_evidence") || s.contains("data-evidence") {
        "Data & Evidence"
    } else if s.contains("methodology") {
        "Methodology Scientist"
    } else if s.contains("experiment") {
        "Experiment Engineer"
    } else if s.contains("paper_writer") || s.contains("paper-writer") {
        "Paper Writer"
    } else if s.contains("academic_reviewer") || s.contains("academic-reviewer") {
        "Academic Reviewer"
    } else if s == "mobile-dev" || s == "mobile_dev" {
        "Mobile Dev"
    } else if s.contains("coder") {
        "Coder"
    } else if s.contains("tester") {
        "Tester"
    } else if s.contains("reviewer") || s.contains("review") {
        "Reviewer"
    } else {
        "Planner"
    }
    .into()
}
fn subagent_role(meta: &Value) -> &str {
    let direct = field(meta, "agent_role");
    if !direct.is_empty() {
        return direct;
    }
    let nested = field(&meta["source"]["subagent"]["thread_spawn"], "agent_role");
    if !nested.is_empty() {
        return nested;
    }
    let legacy = field(&meta["source"]["subagent"], "other");
    if !legacy.is_empty() {
        return legacy;
    }
    if meta["source"].get("subagent").is_some() {
        return "unknown_subagent";
    }
    ""
}
fn is_research_role(raw: &str) -> bool {
    let s = raw.to_ascii_lowercase();
    [
        "literature",
        "data_evidence",
        "data-evidence",
        "methodology",
        "experiment",
        "paper_writer",
        "paper-writer",
        "academic_reviewer",
        "academic-reviewer",
    ]
    .iter()
    .any(|x| s.contains(x))
}
fn is_monitored_subagent(raw: &str) -> bool {
    raw.is_empty()
        || ["planner", "coder", "mobile-dev", "mobile_dev", "tester", "reviewer"]
            .contains(&raw.to_ascii_lowercase().as_str())
        || is_research_role(raw)
}
fn is_research_workspace(raw: &str) -> bool {
    raw.to_ascii_lowercase()
        .replace('\\', "/")
        .contains("research-team")
}
fn text_content(v: &Value) -> String {
    v.as_array()
        .and_then(|a| a.iter().find_map(|x| x.get("text").and_then(Value::as_str)))
        .unwrap_or("")
        .to_owned()
}

fn delegated_role(value: &Value) -> Option<String> {
    for field_name in ["agent_type", "agent_role", "role", "task_name", "agent_name", "target"] {
        let raw = field(value, field_name);
        let leaf = raw
            .rsplit(|c| c == '/' || c == '\\')
            .next()
            .unwrap_or(raw);
        let normalized = leaf.to_ascii_lowercase().replace('_', "-");
        let role = match normalized.as_str() {
            "planner" => "Planner",
            "coder" => "Coder",
            "mobile-dev" => "Mobile Dev",
            "tester" => "Tester",
            "reviewer" => "Reviewer",
            "literature-researcher" => "Literature Researcher",
            "data-evidence" => "Data & Evidence",
            "methodology-scientist" => "Methodology Scientist",
            "experiment-engineer" => "Experiment Engineer",
            "paper-writer" => "Paper Writer",
            "academic-reviewer" => "Academic Reviewer",
            _ => continue,
        };
        return Some(role.into());
    }
    None
}

fn delegated_identity(value: &Value) -> Option<String> {
    for field_name in ["task_name", "agent_name", "target", "agent_role", "role", "agent_type"] {
        let raw = field(value, field_name);
        if raw.is_empty() {
            continue;
        }
        let path = raw.trim().replace('\\', "/").to_ascii_lowercase();
        return Some(if path.starts_with('/') {
            path
        } else {
            format!("/root/{path}")
        });
    }
    None
}

fn task_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with("gAAAA") {
        "Task text unavailable in local log".into()
    } else {
        short(trimmed, 130)
    }
}

fn normalize_agent_status(value: &Value) -> Option<&'static str> {
    let raw = value.as_str().or_else(|| {
        if value.get("completed").is_some_and(|v| !v.is_null()) {
            Some("done")
        } else if value.get("errored").is_some_and(|v| !v.is_null()) {
            Some("failed")
        } else {
            value.get("status").and_then(Value::as_str)
        }
    })?;
    match raw.to_ascii_lowercase().as_str() {
        "idle" => Some("idle"),
        "working" | "running" | "active" => Some("working"),
        "waiting" | "pending" => Some("waiting"),
        "blocked" => Some("blocked"),
        "failed" | "error" | "errored" => Some("failed"),
        "done" | "complete" | "completed" | "finished" => Some("done"),
        _ => None,
    }
}

fn paths_to_scan(paths: Vec<PathBuf>, tracked: &HashMap<PathBuf, u64>) -> Vec<PathBuf> {
    let recent_start = paths.len().saturating_sub(80);
    paths
        .into_iter()
        .enumerate()
        .filter(|(index, path)| *index >= recent_start || tracked.contains_key(path))
        .map(|(_, path)| path)
        .collect()
}

/// Produces a deterministic, non-sensitive identifier for one stored activity row.
/// The source log has no universal event id, so this lets rescans and app restarts
/// preserve history without adding the same metadata event again.
fn event_key(at: &str, thread: &str, role: &str, kind: &str, message: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in [at, thread, role, kind, message].join("\u{1f}").bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

impl Monitor {
    fn new() -> Result<Self, String> {
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
            .ok_or("Cannot locate Codex home")?;
        let app = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("AgentControlCenter");
        fs::create_dir_all(&app).map_err(|e| e.to_string())?;
        Self::open(codex.join("sessions"), app.join("history.sqlite3"))
    }

    fn open(source_dir: PathBuf, db_path: PathBuf) -> Result<Self, String> {
        let db = Connection::open(db_path).map_err(|e| e.to_string())?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, thread_id TEXT NOT NULL, role TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, event_key TEXT); CREATE INDEX IF NOT EXISTS events_at ON events(at);").map_err(|e| e.to_string())?;
        // Existing V1 databases predate event_key. The migration is additive and safe to
        // retry, which lets already-installed users retain their history.
        let _ = db.execute_batch("ALTER TABLE events ADD COLUMN event_key TEXT");
        db.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS events_event_key ON events(event_key) WHERE event_key IS NOT NULL;")
            .map_err(|e| e.to_string())?;
        let next_id = db
            .query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        let mut activity = Vec::new();
        let mut rows = db
            .prepare("SELECT id, at, thread_id, role, kind, text FROM (SELECT id, at, thread_id, role, kind, text FROM events ORDER BY id DESC LIMIT 120) ORDER BY id ASC")
            .and_then(|mut statement| {
                let rows = statement.query_map([], |row| {
                    Ok(Activity {
                        id: row.get(0)?,
                        at: row.get(1)?,
                        thread_id: row.get(2)?,
                        role: row.get(3)?,
                        kind: row.get(4)?,
                        text: row.get(5)?,
                    })
                })?;
                rows.collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| e.to_string())?;
        activity.append(&mut rows);
        Ok(Self {
            files: HashMap::new(),
            ignored: HashSet::new(),
            agents: HashMap::new(),
            session_aliases: HashMap::new(),
            list_agent_calls: HashSet::new(),
            seen_token_records: HashSet::new(),
            agent_token_records: HashSet::new(),
            observed_input_tokens: 0,
            observed_output_tokens: 0,
            activity,
            db,
            next_id,
            source_dir,
        })
    }
    fn record(&mut self, at: &str, thread: &str, role: &str, kind: &str, message: &str) {
        let msg = short(message, 180);
        if msg.is_empty() {
            return;
        }
        let key = event_key(at, thread, role, kind, &msg);
        let inserted = self.db.execute(
            "INSERT OR IGNORE INTO events(at,thread_id,role,kind,text,event_key) VALUES (?1,?2,?3,?4,?5,?6)",
            params![at, thread, role, kind, msg, key],
        );
        if inserted.unwrap_or(0) == 0 {
            return;
        }
        self.next_id = self.db.last_insert_rowid();
        self.activity.push(Activity {
            id: self.next_id,
            at: at.into(),
            thread_id: thread.into(),
            role: role.into(),
            kind: kind.into(),
            text: msg,
        });
        if self.activity.len() > 120 {
            self.activity.drain(..self.activity.len() - 120);
        }
    }

    fn query_activity(&self, filter: ActivityFilter) -> Result<ActivityPage, String> {
        let permitted_roles = roles_for_team(&filter.team)?;
        if let Some(role) = filter.role.as_deref() {
            if !permitted_roles.contains(&role) {
                return Err("The selected role does not belong to the selected team".into());
            }
        }
        let limit = filter.limit.unwrap_or(50).clamp(1, 100);
        let mut predicates = vec![format!(
            "role IN ({})",
            std::iter::repeat("?").take(permitted_roles.len()).collect::<Vec<_>>().join(",")
        )];
        let mut values: Vec<SqlValue> = permitted_roles
            .into_iter()
            .map(|role| SqlValue::Text(role.into()))
            .collect();
        if let Some(role) = filter.role.filter(|role| !role.is_empty()) {
            predicates.push("role = ?".into());
            values.push(SqlValue::Text(role));
        }
        if let Some(kind) = filter.kind.filter(|kind| !kind.is_empty()) {
            predicates.push("kind = ?".into());
            values.push(SqlValue::Text(kind));
        }
        if let Some(query) = filter.query.filter(|query| !query.is_empty()) {
            predicates.push("LOWER(text) LIKE LOWER(?) ESCAPE '\\'".into());
            values.push(SqlValue::Text(format!("%{}%", escape_like(&query))));
        }
        if let Some(before_id) = filter.before_id.filter(|id| *id > 0) {
            predicates.push("id < ?".into());
            values.push(SqlValue::Integer(before_id));
        }
        values.push(SqlValue::Integer(limit + 1));
        let sql = format!(
            "SELECT id, at, thread_id, role, kind, text FROM events WHERE {} ORDER BY id DESC LIMIT ?",
            predicates.join(" AND ")
        );
        let mut statement = self.db.prepare(&sql).map_err(|e| e.to_string())?;
        let mut events = statement
            .query_map(params_from_iter(values.iter()), |row| {
                Ok(Activity {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    thread_id: row.get(2)?,
                    role: row.get(3)?,
                    kind: row.get(4)?,
                    text: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let has_more = events.len() > limit as usize;
        events.truncate(limit as usize);
        for event in &mut events {
            event.thread_id = self.visible_thread_id(&event.thread_id);
        }
        Ok(ActivityPage { events, has_more })
    }

    /// Event rows retain their internal ingestion key for stable deduplication. Resolve
    /// it only when returning a response, so activity and workflow cards share the
    /// public Codex session id without rewriting historical SQLite rows.
    fn visible_thread_id(&self, raw_thread_id: &str) -> String {
        self.agents
            .get(raw_thread_id)
            .or_else(|| {
                self.session_aliases
                    .get(raw_thread_id)
                    .and_then(|internal| self.agents.get(internal))
            })
            .map(|agent| agent.thread_id.clone())
            .filter(|thread_id| !thread_id.is_empty())
            .unwrap_or_else(|| raw_thread_id.into())
    }

    fn observe_delegation(&mut self, parent_id: &str, identity: &str, role: &str, at: &str, task: &str, detail: &str) {
        let Some(parent) = self.agents.get(parent_id) else {
            return;
        };
        let key = format!("{parent_id}::delegated::{identity}");
        let team = parent.team.clone();
        let workspace = parent.workspace.clone();
        let child = self.agents.entry(key.clone()).or_insert_with(|| AgentState {
            team,
            role: role.into(),
            status: "waiting".into(),
            workspace,
            parent_id: parent_id.into(),
            ..Default::default()
        });
        child.parent_id = parent_id.into();
        // A dispatch confirms that a child was requested, but it does not prove the
        // child is still running. list_agents is the authoritative lifecycle source.
        child.status = "waiting".into();
        child.updated_at = at.into();
        if child.started_at.is_empty() || detail.starts_with("Follow-up") {
            child.started_at = at.into();
        }
        if !task.is_empty() {
            child.task = task_text(task);
        }
        child.detail = detail.into();
        self.record(at, &key, role, "delegation", "Delegated agent observed");
    }

    fn observe_agent_statuses(&mut self, parent_id: &str, value: &Value, at: &str) {
        let Some(agents) = value.get("agents").and_then(Value::as_array) else {
            return;
        };
        for item in agents {
            let Some(identity) = delegated_identity(item) else {
                continue;
            };
            let status = item
                .get("agent_status")
                .and_then(normalize_agent_status);
            let Some(status) = status else {
                continue;
            };
            let key = format!("{parent_id}::delegated::{identity}");
            let Some(child) = self.agents.get_mut(&key) else {
                continue;
            };
            child.status = status.into();
            child.updated_at = at.into();
            child.detail = "Observed by list_agents".into();
            let role = child.role.clone();
            self.record(at, &key, &role, "status", &format!("Agent status: {status}"));
        }
    }
    fn ingest(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let at = field(&v, "timestamp").to_owned();
        let typ = field(&v, "type");
        let p = &v["payload"];
        if typ == "session_meta" {
            let session_id = field(p, "id");
            let key = field(&v, "_monitor_thread");
            if session_id.is_empty() && key.is_empty() {
                return;
            }
            let subagent = subagent_role(p);
            if !is_monitored_subagent(subagent) {
                return;
            }
            let workspace = field(p, "cwd");
            let team = if is_research_role(subagent) || is_research_workspace(workspace) {
                "research"
            } else {
                "software"
            };
            let role = if subagent.is_empty() && team == "research" {
                "Research Lead".into()
            } else {
                normalize_role(subagent)
            };
            // Rollout records often omit thread_id after session_meta. Use the file UUID
            // as the internal key so those later records update this same explicit role.
            let key = if key.is_empty() { session_id } else { key };
            if !session_id.is_empty() {
                self.session_aliases.insert(session_id.into(), key.into());
            }
            let a = self.agents.entry(key.to_owned()).or_default();
            a.thread_id = if session_id.is_empty() { key } else { session_id }.into();
            a.team = team.into();
            a.role = role.clone();
            a.workspace = workspace.into();
            a.started_at = at.clone();
            a.updated_at = at.clone();
            if a.status.is_empty() {
                a.status = "idle".into();
            }
            self.record(&at, key, &role, "session", "Session discovered");
            return;
        }
        let thread = field(p, "thread_id");
        let id = if thread.is_empty() {
            field(p, "session_id")
        } else {
            thread
        };
        // Most rollout entries omit thread_id; the caller supplies a synthetic file context.
        let id = if id.is_empty() {
            field(&v, "_monitor_thread")
        } else {
            id
        };
        if id.is_empty() {
            return;
        }
        let id = self
            .session_aliases
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.into());
        // A session meta record is required before an event is assigned to a role. Root
        // sessions intentionally become Planner; internal subagent roles are filtered
        // above, so guardian events never occupy the Planner card.
        let Some(role) = self.agents.get(&id).map(|a| a.role.clone()) else {
            return;
        };
        let Some(a) = self.agents.get_mut(&id) else {
            return;
        };
        a.updated_at = at.clone();
        let mut event: Option<(&str, String)> = None;
        let mut delegated: Option<(String, Option<String>, String, String)> = None;
        let mut list_agent_call_id: Option<String> = None;
        let mut agent_status_output: Option<(String, Value)> = None;
        let mut token_record: Option<(String, u64, u64)> = None;
        match typ {
            "turn_context" => {
                let m = field(p, "model");
                // Parent rollouts multiplex child turns but do not identify the owner of
                // turn_context. Do not overwrite the Planner with a child model.
                if a.role != "Planner" && !m.is_empty() {
                    a.model = m.into();
                }
            }
            "token_usage_record" => {
                let u = &p["usage"];
                let response_id = field(p, "response_id");
                let record_key = if !response_id.is_empty() {
                    format!("response\u{1f}{response_id}")
                } else {
                    let session = field(p, "session_id");
                    let session = if session.is_empty() { &id } else { session };
                    // Codex can emit several usage records for one turn. Without a
                    // response id, timestamp and counters distinguish those records
                    // while remaining stable when the same JSONL entry is replayed.
                    format!(
                        "stream\u{1f}{session}\u{1f}{}\u{1f}{at}\u{1f}{}\u{1f}{}",
                        field(p, "turn_id"),
                        u["input_tokens"],
                        u["output_tokens"]
                    )
                };
                token_record = Some((
                    record_key,
                    u["input_tokens"].as_u64().unwrap_or(0),
                    u["output_tokens"].as_u64().unwrap_or(0),
                ));
            }
            "event_msg" => match field(p, "type") {
                "task_started" => {
                    a.status = "working".into();
                    a.started_at = at.clone();
                    event = Some(("turn", "Turn started".into()));
                }
                "task_complete" | "turn_complete" => {
                    a.status = "done".into();
                    event = Some(("turn", "Turn completed".into()));
                }
                "task_failed" => {
                    a.status = "failed".into();
                    event = Some(("error", "Turn failed".into()));
                }
                "turn_aborted" => {
                    a.status = "blocked".into();
                    event = Some(("turn", "Turn interrupted".into()));
                }
                "request_user_input" | "approval_requested" => {
                    a.status = "blocked".into();
                    event = Some(("approval", "Waiting for user action".into()));
                }
                "agent_message" => {
                    let msg = field(p, "message");
                    if !msg.is_empty() {
                        a.detail = short(msg, 100);
                        event = Some(("message", "Agent update".into()));
                    }
                }
                "user_message" => {
                    let msg = field(p, "message");
                    if !msg.is_empty() {
                        a.task = short(msg, 130);
                    }
                }
                "item_started" | "item_completed" => {
                    let item = &p["item"];
                    let kind = field(item, "type");
                    let normalized_kind = normalized_item_type(item);
                    if normalized_kind == "commandexecution" {
                        a.command = short(field(item, "command"), 100);
                        a.detail = a.command.clone();
                    }
                    if normalized_kind == "filechange" {
                        let path = field(item, "path");
                        let path = if path.is_empty() {
                            item["changes"]
                                .as_array()
                                .and_then(|x| x.first())
                                .map(|x| field(x, "path"))
                                .unwrap_or("")
                        } else {
                            path
                        };
                        a.current_file = short(path, 120);
                        a.detail = a.current_file.clone();
                    }
                    if normalized_kind == "agentmessage" {
                        a.detail = "Agent message".into();
                    }
                    if field(p, "type") == "item_started" && !kind.is_empty() {
                        event = Some(("item", format!("{} started", kind)));
                    }
                }
                "token_count" => {}
                _ => {}
            },
            "response_item" => match field(p, "type") {
                "message" if field(p, "role") == "user" => {
                    let t = text_content(&p["content"]);
                    if !t.is_empty() {
                        a.task = short(&t, 130);
                    }
                }
                "custom_tool_call" | "function_call" => {
                    let name = field(p, "name");
                    a.detail = short(name, 100);
                    if name == "spawn_agent" || name == "followup_task" {
                        if let Ok(args) = serde_json::from_str::<Value>(field(p, "arguments")) {
                            if let Some(identity) = delegated_identity(&args) {
                                let detail = if name == "followup_task" {
                                    "Follow-up dispatched · status unavailable"
                                } else {
                                    "Dispatched · status unavailable"
                                };
                                delegated = Some((identity, delegated_role(&args), field(&args, "message").into(), detail.into()));
                            }
                        }
                    }
                    if name == "list_agents" {
                        let call_id = field(p, "call_id");
                        if !call_id.is_empty() {
                            list_agent_call_id = Some(call_id.into());
                        }
                    }
                    if name.contains("wait_agent") || name.contains("wait_threads") {
                        a.status = "waiting".into();
                    }
                    event = Some(("tool", format!("Tool: {}", name)));
                }
                "custom_tool_call_output" | "function_call_output" => {
                    if a.status == "waiting" {
                        a.status = "working".into();
                    }
                    let call_id = field(p, "call_id");
                    if !call_id.is_empty() {
                        if let Ok(output) = serde_json::from_str(field(p, "output")) {
                            agent_status_output = Some((call_id.into(), output));
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        if let Some((kind, message)) = event {
            self.record(&at, &id, &role, kind, &message);
        }
        if let Some(call_id) = list_agent_call_id {
            self.list_agent_calls.insert(format!("{id}\u{1f}{call_id}"));
        }
        if let Some((identity, child_role, task, detail)) = delegated {
            let child_role = child_role.or_else(|| {
                self.agents
                    .get(&format!("{id}::delegated::{identity}"))
                    .map(|child| child.role.clone())
            });
            if let Some(child_role) = child_role {
                self.observe_delegation(&id, &identity, &child_role, &at, &task, &detail);
            }
        }
        if let Some((call_id, statuses)) = agent_status_output {
            if self.list_agent_calls.contains(&format!("{id}\u{1f}{call_id}")) {
                self.observe_agent_statuses(&id, &statuses, &at);
            }
        }
        if let Some((record_key, input, output)) = token_record {
            if self.seen_token_records.insert(record_key.clone()) {
                self.observed_input_tokens += input;
                self.observed_output_tokens += output;
            }
            let agent_key = format!("{id}\u{1e}{record_key}");
            if role != "Planner" && self.agent_token_records.insert(agent_key) {
                if let Some(agent) = self.agents.get_mut(&id) {
                    agent.input_tokens += input;
                    agent.output_tokens += output;
                    agent.tokens_observed = true;
                }
            }
        }
    }
    fn scan(&mut self) {
        if !self.source_dir.exists() {
            return;
        }
        let mut paths: Vec<_> = WalkDir::new(&self.source_dir)
            .max_depth(4)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "jsonl")
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        paths.sort_by_key(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH)
        });
        // Bound the initial import to recent sessions while continuing to follow any
        // previously observed rollout, even if more than 80 newer files now exist.
        for path in paths_to_scan(paths, &self.files) {
            if self.ignored.contains(&path) {
                continue;
            }
            let Ok(mut f) = File::open(&path) else {
                continue;
            };
            let context = path
                .file_stem()
                .and_then(|x| x.to_str())
                .and_then(|stem| stem.get(stem.len().saturating_sub(36)..))
                .filter(|id| id.len() == 36)
                .unwrap_or("")
                .to_owned();
            let offset = *self.files.get(&path).unwrap_or(&0);
            let Ok(len) = f.metadata().map(|m| m.len()) else {
                continue;
            };
            if len < offset {
                self.files.insert(path.clone(), 0);
                self.agents.remove(&context);
                self.session_aliases.retain(|_, key| key != &context);
                let token_prefix = format!("{context}\u{1e}");
                self.agent_token_records.retain(|key| !key.starts_with(&token_prefix));
                let list_calls_prefix = format!("{context}\u{1f}");
                self.list_agent_calls
                    .retain(|key| !key.starts_with(&list_calls_prefix));
            }
            let pos = *self.files.get(&path).unwrap_or(&0);
            if f.seek(SeekFrom::Start(pos)).is_err() {
                continue;
            }
            let mut reader = BufReader::new(f);
            let mut position = pos;
            let mut line = String::new();
            // Classify from the first record before processing any other events.
            if pos == 0 {
                let mut first = String::new();
                if reader.read_line(&mut first).is_ok() {
                    if let Ok(meta) = serde_json::from_str::<Value>(&first) {
                        let sub = subagent_role(&meta["payload"]).to_ascii_lowercase();
                        if !is_monitored_subagent(&sub) {
                            self.ignored.insert(path);
                            continue;
                        }
                    }
                }
                let _ = reader.seek(SeekFrom::Start(0));
            }
            loop {
                line.clear();
                let Ok(bytes) = reader.read_line(&mut line) else {
                    break;
                };
                if bytes == 0 {
                    break;
                };
                if !line.ends_with('\n') {
                    break;
                }
                position += bytes as u64;
                if let Ok(mut v) = serde_json::from_str::<Value>(&line) {
                    v["_monitor_thread"] = Value::String(context.clone());
                    self.ingest(&v.to_string());
                }
            }
            self.files.insert(path, position);
        }
    }
    fn snapshot(&self) -> Snapshot {
        let mut agents: Vec<_> = self.agents.values().cloned().collect();
        agents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        // A card shows the most recently active session of each configured role in each profile.
        let mut cards = Vec::new();
        for (team, role) in [
            ("software", "Planner"),
            ("software", "Coder"),
            ("software", "Mobile Dev"),
            ("software", "Tester"),
            ("software", "Reviewer"),
            ("research", "Research Lead"),
            ("research", "Literature Researcher"),
            ("research", "Data & Evidence"),
            ("research", "Methodology Scientist"),
            ("research", "Experiment Engineer"),
            ("research", "Paper Writer"),
            ("research", "Academic Reviewer"),
        ] {
            let role_agents: Vec<_> = agents
                .iter()
                .filter(|a| a.team == team && a.role == role)
                .collect();
            let newest = role_agents.iter().copied().max_by(|a, b| {
                a.updated_at
                    .cmp(&b.updated_at)
                    .then_with(|| (!a.thread_id.is_empty()).cmp(&(!b.thread_id.is_empty())))
            });
            let Some(newest) = newest else {
                cards.push(AgentState {
                    team: team.into(),
                    role: role.into(),
                    status: "idle".into(),
                    ..Default::default()
                });
                continue;
            };
            // A direct session forms its own freshest observation. A delegated
            // session shares a cohort with sibling tasks from the same parent.
            let cohort: Vec<_> = if newest.parent_id.is_empty() {
                vec![newest]
            } else {
                role_agents
                    .iter()
                    .copied()
                    .filter(|a| a.parent_id == newest.parent_id)
                    .collect()
            };
            let active_agents: Vec<_> = cohort
                .iter()
                .copied()
                .filter(|a| a.status == "working")
                .collect();
            let selection = if active_agents.is_empty() { &cohort } else { &active_agents };
            if let Some(a) = selection.iter().copied().max_by(|a, b| {
                a.updated_at
                    .cmp(&b.updated_at)
                    .then_with(|| (!a.thread_id.is_empty()).cmp(&(!b.thread_id.is_empty())))
            }) {
                let mut card = a.clone();
                let delegated: Vec<_> = cohort.iter().filter(|a| !a.parent_id.is_empty()).collect();
                if delegated.len() > 1 {
                    let working = delegated.iter().filter(|a| a.status == "working").count();
                    let summary = format!("{working} working / {} observed tasks", delegated.len());
                    card.detail = if card.detail.is_empty() {
                        summary
                    } else {
                        format!("{summary} · {}", card.detail)
                    };
                }
                cards.push(card);
            }
        }
        let total_input = self.observed_input_tokens;
        let total_output = self.observed_output_tokens;
        let research_root = std::env::var_os("AGENT_CONTROL_RESEARCH_WORKSPACE")
            .map(PathBuf::from)
            .or_else(|| {
                agents
                    .iter()
                    .find(|a| a.team == "research" && !a.workspace.is_empty())
                    .map(|a| PathBuf::from(&a.workspace))
            });
        let research = research_root.map(research_metrics).unwrap_or_default();
        let source_available = self.source_dir.is_dir();
        Snapshot {
            connection: if source_available { "connected" } else { "disconnected" }.into(),
            source: if source_available {
                "Codex local session events (read only)".into()
            } else {
                format!("Codex session directory is unavailable: {}", self.source_dir.display())
            },
            agents: cards,
            activity: self
                .activity
                .iter()
                .rev()
                .take(80)
                .cloned()
                .map(|mut event| {
                    event.thread_id = self.visible_thread_id(&event.thread_id);
                    event
                })
                .collect(),
            research,
            total_input,
            total_output,
            updated_at: timestamp(),
        }
    }
}

fn research_metrics(root: PathBuf) -> ResearchMetrics {
    let mut m = ResearchMetrics {
        workspace: root.to_string_lossy().into(),
        review_status: "NOT_REVIEWED".into(),
        ..Default::default()
    };
    if let Ok(s) = fs::read_to_string(root.join("literature/literature_matrix.csv")) {
        let mut lines = s.lines();
        let headers: Vec<_> = lines.next().unwrap_or("").split(',').collect();
        let screen = headers
            .iter()
            .position(|x| *x == "screening_status")
            .unwrap_or(12);
        let verify = headers
            .iter()
            .position(|x| *x == "verification_status")
            .unwrap_or(13);
        for row in lines.map(|x| x.split(',').collect::<Vec<_>>()).filter(|r| {
            r.first()
                .is_some_and(|id| !id.is_empty() && *id != "LIT-000")
        }) {
            m.papers_found += 1;
            let status = row.get(screen).copied().unwrap_or("").to_ascii_lowercase();
            if status != "found" && !status.is_empty() {
                m.papers_screened += 1;
            }
            if status == "included" {
                m.papers_included += 1;
            }
            let v = row.get(verify).copied().unwrap_or("").to_ascii_lowercase();
            if v.contains("verified") && !v.contains("not_verified") {
                m.citations_verified += 1;
            } else if v.contains("broken") {
                m.citations_broken += 1;
            } else {
                m.citations_unverified += 1;
            }
        }
    }
    if let Ok(s) = fs::read_to_string(root.join("data/provenance.md")) {
        m.datasets = s
            .lines()
            .filter(|x| x.contains("| DATA-") && !x.contains("DATA-000"))
            .count() as u64;
    }
    let cfg = root.join("experiments/configs");
    if cfg.exists() {
        m.experiments_complete = WalkDir::new(cfg)
            .max_depth(2)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                fs::read_to_string(e.path()).is_ok_and(|s| s.contains("status = \"complete\""))
            })
            .count() as u64;
    }
    let sections = root.join("paper/sections");
    if sections.exists() {
        for e in WalkDir::new(sections)
            .max_depth(1)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "tex"))
        {
            m.sections_total += 1;
            if fs::read_to_string(e.path()).is_ok_and(|s| {
                !s.contains("NEED_SOURCE:")
                    && !s.contains("NEED_EXPERIMENT:")
                    && !s.contains("SOURCE_NOT_VERIFIED")
            }) {
                m.sections_complete += 1;
            }
        }
    }
    if let Ok(s) = fs::read_to_string(root.join("paper/review_status.md")) {
        if let Some(x) = s.lines().find(|x| x.starts_with("- Status:")) {
            m.review_status = x.trim_start_matches("- Status:").trim().into();
        }
    }
    m
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, Mutex<Monitor>>) -> Result<Snapshot, String> {
    let monitor = state.lock().map_err(|e| e.to_string())?;
    Ok(monitor.snapshot())
}

#[tauri::command]
fn query_activity(
    filter: ActivityFilter,
    state: tauri::State<'_, Mutex<Monitor>>,
) -> Result<ActivityPage, String> {
    state
        .lock()
        .map_err(|e| e.to_string())?
        .query_activity(filter)
}

#[tauri::command]
fn get_model_settings(
    state: tauri::State<'_, Mutex<settings::SettingsService>>,
) -> Result<settings::ModelSettings, String> {
    Ok(state.lock().map_err(|e| e.to_string())?.get())
}

#[tauri::command]
fn save_agent_model(
    role_key: String,
    model: String,
    effort: String,
    revision: String,
    state: tauri::State<'_, Mutex<settings::SettingsService>>,
) -> Result<settings::SaveModelResult, String> {
    state
        .lock()
        .map_err(|e| e.to_string())?
        .save(&role_key, &model, &effort, &revision)
}

pub fn run() {
    let monitor = Monitor::new().expect("Monitor initialization failed");
    let model_settings = settings::SettingsService::new().expect("Model settings initialization failed");
    tauri::Builder::default()
        .manage(Mutex::new(monitor))
        .manage(Mutex::new(model_settings))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            query_activity,
            get_model_settings,
            save_agent_model
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                if let Some(state) = handle.try_state::<Mutex<Monitor>>() {
                    if let Ok(mut m) = state.lock() {
                        m.scan();
                        let _ = handle.emit("monitor-update", m.snapshot());
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri application failed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn activity_monitor() -> Monitor {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, thread_id TEXT NOT NULL, role TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, event_key TEXT UNIQUE)").unwrap();
        Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: PathBuf::new(),
        }
    }

    #[test]
    fn activity_query_is_team_scoped_paginated_and_escapes_literal_search() {
        let mut monitor = activity_monitor();
        monitor.record("2026-10-07T00:00:00Z", "software-1", "Coder", "command", "100% ready");
        monitor.record("2026-10-07T00:00:01Z", "research-1", "Paper Writer", "command", "100% research");
        monitor.record("2026-10-07T00:00:02Z", "software-2", "Tester", "test", "100x ready");
        monitor.record("2026-10-07T00:00:03Z", "software-3", "Coder", "command", "compile complete");

        let first = monitor.query_activity(ActivityFilter { team: "software".into(), role: None, kind: None, query: None, before_id: None, limit: Some(2) }).unwrap();
        assert_eq!(first.events.iter().map(|event| event.id).collect::<Vec<_>>(), vec![4, 3]);
        assert!(first.has_more);
        assert_eq!(first.events[0].thread_id, "software-3");

        let second = monitor.query_activity(ActivityFilter { team: "software".into(), role: None, kind: None, query: None, before_id: Some(3), limit: Some(2) }).unwrap();
        assert_eq!(second.events.iter().map(|event| event.id).collect::<Vec<_>>(), vec![1]);
        assert!(!second.has_more);

        let literal = monitor.query_activity(ActivityFilter { team: "software".into(), role: Some("Coder".into()), kind: Some("command".into()), query: Some("100%".into()), before_id: None, limit: None }).unwrap();
        assert_eq!(literal.events.len(), 1);
        assert_eq!(literal.events[0].text, "100% ready");
        assert!(monitor.query_activity(ActivityFilter { team: "software".into(), role: Some("Paper Writer".into()), kind: None, query: None, before_id: None, limit: None }).is_err());
    }

    #[test]
    fn activity_visible_thread_matches_workflow_card_for_rollout_file_context() {
        let mut monitor = activity_monitor();
        monitor.ingest(&json!({
            "type":"session_meta", "timestamp":"2026-10-07T00:00:00Z",
            "_monitor_thread":"rollout-file-id",
            "payload":{"id":"actual-session-id", "cwd":"C:/project", "agent_role":"coder"}
        }).to_string());
        monitor.ingest(&json!({
            "type":"event_msg", "timestamp":"2026-10-07T00:00:01Z",
            "_monitor_thread":"rollout-file-id", "payload":{"type":"task_started"}
        }).to_string());

        let snapshot = monitor.snapshot();
        let card = snapshot.agents.into_iter().find(|agent| agent.role == "Coder").unwrap();
        assert_eq!(card.thread_id, "actual-session-id");
        assert!(!snapshot.activity.is_empty());
        assert!(snapshot
            .activity
            .iter()
            .all(|event| event.thread_id == "actual-session-id"));
        let page = monitor.query_activity(ActivityFilter { team: "software".into(), role: Some("Coder".into()), kind: None, query: None, before_id: None, limit: None }).unwrap();
        assert!(!page.events.is_empty());
        assert!(page.events.iter().all(|event| event.thread_id == "actual-session-id"));
    }
    #[test]
    fn roles_accept_current_and_legacy_metadata() {
        for role in [
            "planner",
            "coder",
            "mobile-dev",
            "tester",
            "reviewer",
            "academic_reviewer",
        ] {
            for meta in [
                json!({"agent_role":role}),
                json!({"source":{"subagent":{"thread_spawn":{"agent_role":role}}}}),
                json!({"source":{"subagent":{"other":role}}}),
            ] {
                assert_eq!(subagent_role(&meta), role);
                assert!(is_monitored_subagent(subagent_role(&meta)));
            }
        }
        assert_eq!(normalize_role("mobile-dev"), "Mobile Dev");
        assert_eq!(normalize_role("mobile_dev"), "Mobile Dev");
        assert_eq!(normalize_role("academic_reviewer"), "Academic Reviewer");
        assert_eq!(delegated_role(&json!({"task_name":"tester","agent_type":"coder"})), Some("Coder".into()));
        assert!(!is_monitored_subagent(subagent_role(
            &json!({"source":{"subagent":{"other":"guardian"}}})
        )));
        assert!(!is_monitored_subagent(subagent_role(
            &json!({"source":{"subagent":{"thread_spawn":{}}}})
        )));
        assert_eq!(subagent_role(&json!({"source":"cli"})), "");
        assert!(is_monitored_subagent(""));
    }
    #[test]
    fn mobile_events_reach_software_card_without_changing_research_roster() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(),
            ignored: HashSet::new(),
            agents: HashMap::new(),
            session_aliases: HashMap::new(),
            list_agent_calls: HashSet::new(),
            seen_token_records: HashSet::new(),
            agent_token_records: HashSet::new(),
            observed_input_tokens: 0,
            observed_output_tokens: 0,
            activity: vec![],
            db,
            next_id: 0,
            source_dir: PathBuf::new(),
        };
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-24T00:00:00Z","payload":{"id":"mobile-test","cwd":"C:/mobile","agent_role":"mobile-dev","source":{"subagent":{"thread_spawn":{"agent_role":"mobile-dev"}}}}}).to_string());
        for (typ, payload) in [
            ("turn_context", json!({"model":"gpt-6-sol"})),
            ("event_msg", json!({"type":"task_started"})),
            (
                "event_msg",
                json!({"type":"user_message","message":"Fix Flutter navigation"}),
            ),
            (
                "token_usage_record",
                json!({"usage":{"input_tokens":12,"output_tokens":7}}),
            ),
        ] {
            m.ingest(&json!({"type":typ,"timestamp":"2026-09-24T00:01:00Z","_monitor_thread":"mobile-test","payload":payload}).to_string());
        }
        let snap = m.snapshot();
        assert_eq!(
            snap.agents.iter().filter(|a| a.team == "software").count(),
            5
        );
        assert_eq!(
            snap.agents.iter().filter(|a| a.team == "research").count(),
            7
        );
        let a = snap.agents.iter().find(|a| a.role == "Mobile Dev").unwrap();
        assert_eq!(
            (&*a.model, &*a.status, &*a.task),
            ("gpt-6-sol", "working", "Fix Flutter navigation")
        );
        assert_eq!((a.input_tokens, a.output_tokens), (12, 7));
        assert_eq!(a.started_at, "2026-09-24T00:01:00Z");
    }
    #[test]
    fn pascal_case_item_events_update_mobile_command_and_file() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(),
            ignored: HashSet::new(),
            agents: HashMap::new(),
            session_aliases: HashMap::new(),
            list_agent_calls: HashSet::new(),
            seen_token_records: HashSet::new(),
            agent_token_records: HashSet::new(),
            observed_input_tokens: 0,
            observed_output_tokens: 0,
            activity: vec![],
            db,
            next_id: 0,
            source_dir: PathBuf::new(),
        };
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-24T00:00:00Z","payload":{"id":"mobile-events","cwd":"C:/mobile","agent_role":"mobile-dev"}}).to_string());
        for item in [
            json!({"type":"CommandExecution","command":"flutter test test/navigation_test.dart"}),
            json!({"type":"FileChange","changes":[{"path":"lib/navigation/router.dart"}]}),
        ] {
            m.ingest(&json!({"type":"event_msg","timestamp":"2026-09-24T00:01:00Z","_monitor_thread":"mobile-events","payload":{"type":"item_started","item":item}}).to_string());
        }
        let a = m
            .snapshot()
            .agents
            .into_iter()
            .find(|a| a.role == "Mobile Dev")
            .unwrap();
        assert_eq!(a.command, "flutter test test/navigation_test.dart");
        assert_eq!(a.current_file, "lib/navigation/router.dart");
        assert_eq!(a.detail, "lib/navigation/router.dart");
    }

    #[test]
    fn root_sessions_are_planner_and_guardian_sessions_are_ignored() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: PathBuf::new(),
        };
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-30T00:00:00Z","_monitor_thread":"root-file","payload":{"id":"root-session","cwd":"C:/project","source":"cli"}}).to_string());
        m.ingest(&json!({"type":"turn_context","timestamp":"2026-09-30T00:00:00Z","_monitor_thread":"root-file","payload":{"model":"child-model"}}).to_string());
        m.ingest(&json!({"type":"token_usage_record","timestamp":"2026-09-30T00:00:00Z","_monitor_thread":"root-file","payload":{"usage":{"input_tokens":20,"output_tokens":5}}}).to_string());
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-30T00:00:01Z","_monitor_thread":"guardian-file","payload":{"id":"guardian-session","cwd":"C:/project","source":{"subagent":{"other":"guardian"}}}}).to_string());
        assert_eq!(m.agents.len(), 1);
        let planner = m.agents.get("root-file").unwrap();
        assert_eq!(planner.role, "Planner");
        assert_eq!((&*planner.model, planner.input_tokens, planner.tokens_observed), ("", 0, false));
        let snapshot = m.snapshot();
        assert_eq!((snapshot.total_input, snapshot.total_output), (20, 5));
    }

    #[test]
    fn session_aliases_and_event_keys_keep_token_state_and_history_correct() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: PathBuf::new(),
        };
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-30T00:00:00Z","_monitor_thread":"rollout-file-id","payload":{"id":"actual-session-id","cwd":"C:/project","agent_role":"coder"}}).to_string());
        m.ingest(&json!({"type":"token_usage_record","timestamp":"2026-09-30T00:00:01Z","_monitor_thread":"rollout-file-id","payload":{"thread_id":"actual-session-id","usage":{"input_tokens":12,"output_tokens":7}}}).to_string());
        let coder = m.agents.get("rollout-file-id").unwrap();
        assert_eq!((coder.input_tokens, coder.output_tokens), (12, 7));
        m.record("2026-09-30T00:00:02Z", "actual-session-id", "Coder", "turn", "Turn started");
        m.record("2026-09-30T00:00:02Z", "actual-session-id", "Coder", "turn", "Turn started");
        assert_eq!(m.activity.iter().filter(|x| x.text == "Turn started").count(), 1);
    }

    #[test]
    fn truncated_rollout_resets_session_before_replay() {
        let root = std::env::temp_dir().join(format!("agent-control-center-test-{}-{}", std::process::id(), timestamp()));
        fs::create_dir_all(&root).unwrap();
        let session = "11111111-1111-1111-1111-111111111111";
        let path = root.join(format!("rollout-{session}.jsonl"));
        let meta = json!({"type":"session_meta","timestamp":"2026-09-30T00:00:00Z","payload":{"id":"actual-session","cwd":"C:/project","agent_role":"coder"}});
        let tokens = json!({"type":"token_usage_record","timestamp":"2026-09-30T00:00:01Z","payload":{"usage":{"input_tokens":10,"output_tokens":4}}});
        let extra = json!({"type":"event_msg","timestamp":"2026-09-30T00:00:02Z","payload":{"type":"task_started"}});
        fs::write(&path, format!("{meta}\n{tokens}\n{extra}\n")).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: root.clone(),
        };
        m.scan();
        m.list_agent_calls.insert(format!("{session}\u{1f}stale-list-call"));
        fs::write(&path, format!("{meta}\n{tokens}\n")).unwrap();
        m.scan();
        let agent = m.agents.get(session).unwrap();
        assert_eq!((agent.input_tokens, agent.output_tokens), (10, 4));
        assert!(m.list_agent_calls.is_empty());
        m.agents.insert(format!("{session}::delegated::/root/coder"), AgentState {
            team: "software".into(), role: "Coder".into(), parent_id: session.into(),
            status: "waiting".into(), ..Default::default()
        });
        m.ingest(&json!({
            "type":"response_item", "timestamp":"2026-09-30T00:00:03Z", "_monitor_thread":session,
            "payload":{"type":"function_call_output", "call_id":"stale-list-call", "output":"{\"agents\":[{\"agent_name\":\"/root/coder\",\"agent_status\":\"done\"}]}"}
        }).to_string());
        assert_eq!(m.agents.get(&format!("{session}::delegated::/root/coder")).unwrap().status, "waiting");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reopening_sqlite_rehydrates_recent_activity_without_replay_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "agent-control-center-history-{}-{}",
            std::process::id(),
            timestamp()
        ));
        fs::create_dir_all(&root).unwrap();
        let db_path = root.join("history.sqlite3");
        {
            let mut first = Monitor::open(root.join("sessions"), db_path.clone()).unwrap();
            first.record("2026-09-30T00:00:00Z", "thread-1", "Coder", "turn", "Turn started");
            assert_eq!(first.activity.len(), 1);
        }
        let mut reopened = Monitor::open(root.join("sessions"), db_path).unwrap();
        assert_eq!(reopened.activity.len(), 1);
        assert_eq!(reopened.activity[0].text, "Turn started");
        reopened.record("2026-09-30T00:00:00Z", "thread-1", "Coder", "turn", "Turn started");
        assert_eq!(reopened.activity.len(), 1);
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tracked_rollout_is_not_lost_when_more_than_eighty_newer_files_exist() {
        let paths: Vec<PathBuf> = (0..81)
            .map(|index| PathBuf::from(format!("rollout-{index:03}.jsonl")))
            .collect();
        let tracked = HashMap::from([(paths[0].clone(), 42)]);
        let selected = paths_to_scan(paths, &tracked);
        assert_eq!(selected.len(), 81);
        assert!(selected.iter().any(|path| path == &PathBuf::from("rollout-000.jsonl")));
    }

    #[test]
    fn structured_spawn_agent_call_creates_a_real_delegated_role_state() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: PathBuf::new(),
        };
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-30T00:00:00Z","_monitor_thread":"planner-file","payload":{"id":"planner-session","cwd":"C:/project","source":"cli"}}).to_string());
        m.ingest(&json!({
            "type":"response_item", "timestamp":"2026-09-30T00:00:01Z", "_monitor_thread":"planner-file",
            "payload":{"type":"function_call","name":"spawn_agent","arguments":"{\"task_name\":\"/root/model_backend\",\"agent_type\":\"coder\",\"message\":\"gAAAAEncryptedLocalBlob\"}"}
        }).to_string());
        let coder = m.agents.get("planner-file::delegated::/root/model_backend").unwrap();
        assert_eq!((&*coder.role, &*coder.status, &*coder.task), ("Coder", "waiting", "Task text unavailable in local log"));
        assert_eq!(coder.workspace, "C:/project");
        m.ingest(&json!({
            "type":"response_item", "timestamp":"2026-09-30T00:00:02Z", "_monitor_thread":"planner-file",
            "payload":{"type":"function_call_output","call_id":"unrelated","output":"{\"agents\":[{\"agent_name\":\"/root/model_backend\",\"agent_status\":{\"completed\":\"completed\"}}]}"}
        }).to_string());
        assert_eq!(m.agents.get("planner-file::delegated::/root/model_backend").unwrap().status, "waiting");
        m.ingest(&json!({
            "type":"response_item", "timestamp":"2026-09-30T00:00:03Z", "_monitor_thread":"planner-file",
            "payload":{"type":"function_call","name":"list_agents","call_id":"agents-1","arguments":"{}"}
        }).to_string());
        m.ingest(&json!({
            "type":"response_item", "timestamp":"2026-09-30T00:00:04Z", "_monitor_thread":"planner-file",
            "payload":{"type":"function_call_output","call_id":"agents-1","output":"{\"agents\":[{\"agent_name\":\"/root/model_backend\",\"agent_status\":{\"completed\":\"completed\"}}]}"}
        }).to_string());
        assert_eq!(m.agents.get("planner-file::delegated::/root/model_backend").unwrap().status, "done");
        m.ingest(&json!({
            "type":"response_item", "timestamp":"2026-09-30T00:00:05Z", "_monitor_thread":"planner-file",
            "payload":{"type":"function_call","name":"followup_task","arguments":"{\"target\":\"/root/model_backend\",\"message\":\"retry\"}"}
        }).to_string());
        let coder = m.agents.get("planner-file::delegated::/root/model_backend").unwrap();
        assert_eq!((&*coder.status, &*coder.started_at), ("waiting", "2026-09-30T00:00:05Z"));
    }

    #[test]
    fn newer_delegated_state_replaces_an_old_explicit_role_session() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: PathBuf::new(),
        };
        m.agents.insert("explicit".into(), AgentState { team:"software".into(), role:"Coder".into(), thread_id:"coder-session".into(), updated_at:"2026-09-30T00:00:00Z".into(), status:"done".into(), ..Default::default() });
        m.agents.insert("fallback".into(), AgentState { team:"software".into(), role:"Coder".into(), updated_at:"2026-09-30T00:01:00Z".into(), status:"waiting".into(), ..Default::default() });
        let coder = m.snapshot().agents.into_iter().find(|a| a.role == "Coder").unwrap();
        assert_eq!(coder.thread_id, "");
    }

    #[test]
    fn newest_delegated_cohort_beats_stale_working_parent() {
        let mut m = activity_monitor();
        m.agents.insert("parent-a::delegated::/root/old".into(), AgentState {
            team: "software".into(), role: "Coder".into(), parent_id: "parent-a".into(),
            status: "working".into(), updated_at: "2026-10-01T00:00:00Z".into(),
            ..Default::default()
        });
        m.agents.insert("parent-b::delegated::/root/new".into(), AgentState {
            team: "software".into(), role: "Coder".into(), parent_id: "parent-b".into(),
            status: "done".into(), updated_at: "2026-10-01T00:01:00Z".into(),
            ..Default::default()
        });
        let coder = m.snapshot().agents.into_iter().find(|a| a.role == "Coder").unwrap();
        assert_eq!(coder.status, "done");
        assert_eq!(coder.parent_id, "parent-b");
    }

    #[test]
    fn working_sibling_is_preferred_within_newest_delegated_cohort() {
        let mut m = activity_monitor();
        for (identity, status, at) in [
            ("/root/done", "done", "2026-10-01T00:01:01Z"),
            ("/root/working", "working", "2026-10-01T00:01:00Z"),
        ] {
            m.agents.insert(format!("parent-b::delegated::{identity}"), AgentState {
                team: "software".into(), role: "Coder".into(), parent_id: "parent-b".into(),
                status: status.into(), updated_at: at.into(), ..Default::default()
            });
        }
        let coder = m.snapshot().agents.into_iter().find(|a| a.role == "Coder").unwrap();
        assert_eq!(coder.status, "working");
        assert!(coder.detail.starts_with("1 working / 2 observed tasks"));
    }

    #[test]
    fn distinct_usage_records_in_one_turn_count_once_each_and_replay_is_ignored() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE events (id INTEGER PRIMARY KEY, at TEXT, thread_id TEXT, role TEXT, kind TEXT, text TEXT, event_key TEXT UNIQUE)").unwrap();
        let mut m = Monitor {
            files: HashMap::new(), ignored: HashSet::new(), agents: HashMap::new(),
            session_aliases: HashMap::new(), list_agent_calls: HashSet::new(), seen_token_records: HashSet::new(), agent_token_records: HashSet::new(), observed_input_tokens: 0, observed_output_tokens: 0, activity: vec![], db, next_id: 0, source_dir: PathBuf::new(),
        };
        m.ingest(&json!({"type":"session_meta","timestamp":"2026-09-30T00:00:00Z","_monitor_thread":"coder-file","payload":{"id":"coder-session","cwd":"C:/project","agent_role":"coder"}}).to_string());
        for (at, input, output) in [("2026-09-30T00:00:01Z", 10, 2), ("2026-09-30T00:00:02Z", 15, 3)] {
            let record = json!({"type":"token_usage_record","timestamp":at,"_monitor_thread":"coder-file","payload":{"session_id":"coder-session","turn_id":"one-turn","usage":{"input_tokens":input,"output_tokens":output}}}).to_string();
            m.ingest(&record);
            if input == 10 { m.ingest(&record); }
        }
        let snapshot = m.snapshot();
        assert_eq!((snapshot.total_input, snapshot.total_output), (25, 5));
        let coder = snapshot.agents.into_iter().find(|a| a.role == "Coder").unwrap();
        assert_eq!((coder.input_tokens, coder.output_tokens), (25, 5));
    }
}
