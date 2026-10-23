use crate::app_server::{AppServerEvent, HumanRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_TASKS: usize = 50;
const MAX_EVENTS: usize = 500;
const MAX_EVENT_MESSAGE_CHARS: usize = 320;
const MAX_STREAM_TRACKERS: usize = 128;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotLimits {
    pub max_attempts: u32,
    pub max_runtime_minutes: u64,
    pub max_tokens: u64,
    pub max_parallel: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskInput {
    pub id: String,
    pub title: String,
    pub description: String,
    pub role: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotPlanInput {
    #[serde(default)]
    pub plan_id: Option<String>,
    pub title: String,
    pub objective: String,
    pub workspace: String,
    pub limits: AutopilotLimits,
    pub tasks: Vec<PlanTaskInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTask {
    pub id: String,
    pub title: String,
    pub description: String,
    pub role: String,
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    pub status: String,
    pub attempts: u32,
    pub thread_id: String,
    pub turn_id: String,
    pub summary: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub active_input_tokens: u64,
    #[serde(default)]
    pub active_output_tokens: u64,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub feedback: String,
    #[serde(default)]
    pub human_requests: Vec<HumanRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotPlan {
    pub plan_id: String,
    pub title: String,
    pub objective: String,
    pub workspace: String,
    pub limits: AutopilotLimits,
    pub tasks: Vec<PlanTask>,
    pub revision: u64,
    #[serde(default)]
    pub approved_revision: Option<u64>,
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    #[serde(default)]
    pub run_started_at: Option<u64>,
    #[serde(default)]
    pub pause_requested: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotEvent {
    pub id: u64,
    pub at: u64,
    pub kind: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRunIdentity {
    pub plan_id: String,
    pub revision: u64,
    pub task_id: String,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotSnapshot {
    #[serde(default)]
    pub plan: Option<AutopilotPlan>,
    pub execution_available: bool,
    pub connection_message: String,
    #[serde(default)]
    pub events: Vec<AutopilotEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedAutopilot {
    plan: Option<AutopilotPlan>,
    events: Vec<AutopilotEvent>,
    next_event_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StructuredOutcome {
    Pass(String),
    NeedsInput(String),
    Fail(String),
}

/// Durable state machine for user-approved Software Team runs. It deliberately
/// has no dependency on the monitor: only the app-server worker may call
/// `complete_turn`, and only after receiving a terminal turn notification.
pub struct AutopilotController {
    path: PathBuf,
    state: PersistedAutopilot,
    execution_available: bool,
    connection_message: String,
    persistence_blocked: bool,
    /// Ephemeral pointers into the bounded persisted event list. Stream state is
    /// deliberately not serialized; after restart, the next delta starts a new
    /// concise row instead of retaining raw app-server payloads.
    stream_events: HashMap<StreamEventKey, u64>,
    stream_previews: HashMap<StreamEventKey, String>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct StreamEventKey {
    task_id: String,
    turn_id: String,
    item_id: String,
    family: String,
}

enum NormalizedTransportEvent {
    Drop,
    Add {
        kind: &'static str,
        message: String,
        stream_key: Option<StreamEventKey>,
    },
    Stream {
        kind: &'static str,
        key: StreamEventKey,
        prefix: &'static str,
        delta: String,
    },
    FinishTurn {
        kind: &'static str,
        message: String,
        turn_id: String,
    },
}

impl AutopilotController {
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let mut state = if path.exists() {
            let bytes = fs::read(&path).map_err(|e| format!("Cannot read autopilot state: {e}"))?;
            serde_json::from_slice(&bytes)
                .map_err(|e| format!("Autopilot state is invalid; it was left untouched: {e}"))?
        } else {
            PersistedAutopilot::default()
        };
        let mut changed = false;
        if let Some(plan) = state.plan.as_mut() {
            let has_live_request = plan
                .tasks
                .iter()
                .any(|task| task.status == "running" || !task.human_requests.is_empty());
            if matches!(plan.status.as_str(), "running" | "paused" | "cancelling")
                || has_live_request
            {
                plan.status = "interrupted".into();
                plan.updated_at = now();
                for task in &mut plan.tasks {
                    if task.status == "running" || !task.human_requests.is_empty() {
                        task.status = "needsInput".into();
                        task.summary = "The app restarted while this task was attached to the previous Codex App Server. Its request ids are invalid; choose Retry to start a fresh bounded attempt.".into();
                        task.finished_at = Some(now());
                        task.human_requests.clear();
                    }
                }
                state.next_event_id += 1;
                state.events.push(AutopilotEvent {
                    id: state.next_event_id,
                    at: now(),
                    kind: "recovery".into(),
                    task_id: None,
                    message:
                        "An unfinished run was recovered as interrupted. Review it before retrying."
                            .into(),
                });
                changed = true;
            }
        }
        let mut controller = Self {
            path,
            state,
            execution_available: false,
            connection_message: "Codex App Server has not been probed yet".into(),
            persistence_blocked: false,
            stream_events: HashMap::new(),
            stream_previews: HashMap::new(),
        };
        if changed {
            controller.persist()?;
        }
        Ok(controller)
    }

    pub fn new_default() -> Result<Self, String> {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("AgentControlCenter");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Self::new(dir.join("autopilot.json"))
    }

    pub fn snapshot(&self) -> AutopilotSnapshot {
        AutopilotSnapshot {
            plan: self.state.plan.clone(),
            execution_available: self.execution_available,
            connection_message: self.connection_message.clone(),
            events: self.state.events.clone(),
        }
    }

    pub fn set_connection(&mut self, available: bool, message: impl Into<String>) {
        if self.persistence_blocked {
            return;
        }
        self.execution_available = available && !self.persistence_blocked;
        self.connection_message = message.into();
    }

    pub fn save_plan(&mut self, input: AutopilotPlanInput) -> Result<AutopilotSnapshot, String> {
        validate_plan_input(&input)?;
        let timestamp = now();
        let previous = self.state.plan.as_ref();
        if previous.is_some_and(|plan| {
            matches!(
                plan.status.as_str(),
                "running" | "paused" | "needsInput" | "cancelling"
            )
        }) {
            return Err("Pause or cancel the active run before editing its plan".into());
        }
        let plan_id = input
            .plan_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("plan-{timestamp}-{}", std::process::id()));
        if let Some(plan) = previous {
            if plan.plan_id != plan_id {
                return Err("The supplied planId does not match the current plan".into());
            }
        }
        let revision = previous.map_or(1, |plan| plan.revision.saturating_add(1));
        let created_at = previous.map_or(timestamp, |plan| plan.created_at);
        let tasks = input
            .tasks
            .into_iter()
            .map(|task| PlanTask {
                id: task.id.trim().into(),
                title: task.title.trim().into(),
                description: task.description.trim().into(),
                role: task.role,
                dependencies: task.dependencies,
                model: task.model.filter(|v| !v.trim().is_empty()),
                effort: task.effort.filter(|v| !v.trim().is_empty()),
                status: "pending".into(),
                attempts: 0,
                thread_id: String::new(),
                turn_id: String::new(),
                summary: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                active_input_tokens: 0,
                active_output_tokens: 0,
                started_at: None,
                finished_at: None,
                feedback: String::new(),
                human_requests: Vec::new(),
            })
            .collect();
        self.state.plan = Some(AutopilotPlan {
            plan_id,
            title: input.title.trim().into(),
            objective: input.objective.trim().into(),
            workspace: input.workspace,
            limits: input.limits,
            tasks,
            revision,
            approved_revision: None,
            status: "draft".into(),
            created_at,
            updated_at: timestamp,
            total_input_tokens: 0,
            total_output_tokens: 0,
            run_started_at: None,
            pause_requested: false,
        });
        self.event(
            "plan",
            None,
            format!("Saved plan revision {revision}; approval is required"),
        );
        self.persist()?;
        Ok(self.snapshot())
    }

    pub fn approve_plan(
        &mut self,
        plan_id: &str,
        revision: u64,
    ) -> Result<AutopilotSnapshot, String> {
        let plan = self.plan_mut(plan_id)?;
        if plan.revision != revision {
            return Err("Plan changed after it was displayed; review the newest revision".into());
        }
        if plan.status != "draft" && plan.status != "interrupted" {
            return Err("Only a draft or recovered plan can be approved".into());
        }
        plan.approved_revision = Some(revision);
        plan.status = "approved".into();
        plan.updated_at = now();
        self.event(
            "approval",
            None,
            format!("Approved plan revision {revision}"),
        );
        self.persist()?;
        Ok(self.snapshot())
    }

    pub fn start(&mut self, plan_id: &str, revision: u64) -> Result<AutopilotSnapshot, String> {
        if !self.execution_available {
            return Err(format!(
                "Codex execution is unavailable: {}",
                self.connection_message
            ));
        }
        let plan = self.plan_mut(plan_id)?;
        if plan.revision != revision || plan.approved_revision != Some(revision) {
            return Err("The current plan revision must be approved before Start".into());
        }
        if plan.status != "approved" {
            return Err("The plan is not ready to start".into());
        }
        plan.status = "running".into();
        plan.pause_requested = false;
        plan.run_started_at = Some(now());
        plan.updated_at = now();
        mark_ready(plan);
        self.event("run", None, "Autopilot started".into());
        self.persist()?;
        Ok(self.snapshot())
    }

    pub fn pause(&mut self, plan_id: &str) -> Result<AutopilotSnapshot, String> {
        let plan = self.plan_mut(plan_id)?;
        if plan.status != "running" {
            return Err("Only a running plan can be paused".into());
        }
        plan.status = "paused".into();
        plan.pause_requested = true;
        plan.updated_at = now();
        self.event(
            "run",
            None,
            "Paused; active work may finish, and no new task will start".into(),
        );
        self.persist()?;
        Ok(self.snapshot())
    }

    pub fn resume(&mut self, plan_id: &str) -> Result<AutopilotSnapshot, String> {
        let plan = self.plan_mut(plan_id)?;
        if plan.status != "paused" && plan.status != "needsInput" {
            return Err("Only a paused or needs-input plan can be resumed".into());
        }
        if plan.tasks.iter().any(|task| task.status == "needsInput") {
            return Err("Resolve all needs-input tasks before resuming".into());
        }
        plan.status = "running".into();
        plan.pause_requested = false;
        plan.updated_at = now();
        mark_ready(plan);
        self.event("run", None, "Autopilot resumed".into());
        self.persist()?;
        Ok(self.snapshot())
    }

    pub fn begin_cancel(&mut self, plan_id: &str) -> Result<Vec<TaskRunIdentity>, String> {
        let plan = self.plan_mut(plan_id)?;
        if matches!(plan.status.as_str(), "completed" | "cancelled") {
            return Err("This run is already terminal".into());
        }
        plan.status = "cancelling".into();
        plan.updated_at = now();
        for task in &mut plan.tasks {
            if !matches!(
                task.status.as_str(),
                "passed" | "failed" | "cancelled" | "running" | "needsInput"
            ) {
                task.status = "cancelled".into();
                task.finished_at = Some(now());
            }
        }
        self.event("run", None, "Autopilot cancellation started".into());
        self.persist()?;
        let plan = self.state.plan.as_ref().ok_or("No autopilot plan")?;
        Ok(plan
            .tasks
            .iter()
            .filter(|task| task.status == "running" || !task.human_requests.is_empty())
            .map(|task| TaskRunIdentity {
                plan_id: plan.plan_id.clone(),
                revision: plan.revision,
                task_id: task.id.clone(),
                thread_id: task.thread_id.clone(),
                turn_id: task.turn_id.clone(),
            })
            .collect())
    }

    pub fn finish_cancel(&mut self, plan_id: &str) -> Result<AutopilotSnapshot, String> {
        let plan = self.plan_mut(plan_id)?;
        if plan.status != "cancelling" {
            return Err("Autopilot is not cancelling".into());
        }
        plan.status = "cancelled".into();
        plan.updated_at = now();
        for task in &mut plan.tasks {
            if !matches!(task.status.as_str(), "passed" | "failed" | "cancelled") {
                task.status = "cancelled".into();
                task.finished_at = Some(now());
                task.human_requests.clear();
            }
        }
        self.event("run", None, "Autopilot cancelled".into());
        self.persist()?;
        Ok(self.snapshot())
    }

    pub fn respond_gate(
        &mut self,
        plan_id: &str,
        task_id: &str,
        response: &str,
        approve: bool,
        retry: bool,
    ) -> Result<AutopilotSnapshot, String> {
        let plan = self.plan_mut(plan_id)?;
        if !matches!(plan.status.as_str(), "needsInput" | "interrupted") {
            return Err("This plan is not accepting human gate responses".into());
        }
        let index = plan
            .tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if plan.tasks[index].status != "needsInput" {
            return Err("This task is not waiting for human input".into());
        }
        if !plan.tasks[index].human_requests.is_empty() {
            return Err(
                "Respond to the pending Codex request explicitly before retrying this task".into(),
            );
        }
        if retry {
            if plan.tasks[index].attempts >= plan.limits.max_attempts {
                return Err("The bounded retry limit has been reached".into());
            }
            reset_task_and_descendants(plan, &plan.tasks[index].id.clone(), response.trim());
            plan.status = if plan.pause_requested {
                "paused"
            } else {
                "running"
            }
            .into();
            mark_ready(plan);
        } else if plan.tasks[index].role == "human" && approve {
            plan.tasks[index].summary = response.trim().into();
            plan.tasks[index].status = "passed".into();
            plan.tasks[index].finished_at = Some(now());
            plan.status = if plan.pause_requested {
                "paused"
            } else {
                "running"
            }
            .into();
            mark_ready(plan);
            update_terminal_status(plan);
        } else {
            plan.tasks[index].summary = response.trim().into();
            plan.tasks[index].status = "failed".into();
            plan.tasks[index].finished_at = Some(now());
            plan.status = "failed".into();
        }
        plan.updated_at = now();
        self.event(
            "human",
            Some(task_id.into()),
            if retry {
                "Retry authorized with human feedback"
            } else if approve {
                "Human gate approved"
            } else {
                "Human gate rejected"
            }
            .into(),
        );
        self.persist()?;
        Ok(self.snapshot())
    }

    /// Returns at most one task because V2 shares a working directory and enforces
    /// serial writes. A worker must call `mark_turn_started` before execution.
    pub fn next_ready_task(&self) -> Option<PlanTask> {
        let plan = self.state.plan.as_ref()?;
        if self.persistence_blocked || plan.status != "running" || caps_exceeded(plan) {
            return None;
        }
        if plan.tasks.iter().any(|task| task.status == "running") {
            return None;
        }
        plan.tasks
            .iter()
            .find(|task| task.status == "ready" && task.role != "human")
            .cloned()
    }

    pub fn activate_human_gate(&mut self) -> Result<bool, String> {
        let Some(plan) = self.state.plan.as_mut() else {
            return Ok(false);
        };
        if plan.status == "running" && caps_exceeded(plan) {
            enforce_caps(plan);
            plan.updated_at = now();
            self.event(
                "limit",
                None,
                "Runtime or token limit reached; dispatch stopped".into(),
            );
            self.persist()?;
            return Ok(true);
        }
        if plan.status != "running" || plan.tasks.iter().any(|task| task.status == "running") {
            return Ok(false);
        }
        let Some(index) = plan
            .tasks
            .iter()
            .position(|task| task.status == "ready" && task.role == "human")
        else {
            return Ok(false);
        };
        plan.tasks[index].status = "needsInput".into();
        plan.tasks[index].summary = "Human direction or final review is required".into();
        plan.status = "needsInput".into();
        let id = plan.tasks[index].id.clone();
        plan.updated_at = now();
        self.event("human", Some(id), "Human gate is ready".into());
        self.persist()?;
        Ok(true)
    }

    pub fn mark_turn_started(
        &mut self,
        task_id: &str,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        if plan.status != "running" {
            return Err("Autopilot is not running".into());
        }
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if task.status != "ready" {
            return Err("Task is not ready".into());
        }
        if task.attempts >= plan.limits.max_attempts {
            return Err("Task retry limit reached".into());
        }
        task.attempts += 1;
        task.status = "running".into();
        task.thread_id = thread_id.into();
        task.turn_id = turn_id.into();
        task.started_at = Some(now());
        task.finished_at = None;
        plan.updated_at = now();
        self.event(
            "dispatch",
            Some(task_id.into()),
            "Task started through Codex App Server".into(),
        );
        self.persist()
    }

    pub fn reserve_task(&mut self, task_id: &str) -> Result<(), String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        if plan.status != "running" {
            return Err("Autopilot is not running".into());
        }
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if task.status != "ready" || task.attempts >= plan.limits.max_attempts {
            return Err("Task is no longer dispatchable".into());
        }
        task.attempts += 1;
        task.status = "running".into();
        task.started_at = Some(now());
        task.finished_at = None;
        task.human_requests.clear();
        task.active_input_tokens = 0;
        task.active_output_tokens = 0;
        plan.updated_at = now();
        self.persist()
    }

    pub fn reserve_task_bound(&mut self, task_id: &str) -> Result<TaskRunIdentity, String> {
        self.reserve_task(task_id)?;
        self.identity_for(task_id)
            .ok_or("Reserved task identity is unavailable".into())
    }

    pub fn attach_turn(
        &mut self,
        task_id: &str,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if task.status != "running" {
            return Err("Task was cancelled before dispatch completed".into());
        }
        task.thread_id = thread_id.into();
        task.turn_id = turn_id.into();
        plan.updated_at = now();
        self.event(
            "dispatch",
            Some(task_id.into()),
            "Task started through Codex App Server".into(),
        );
        self.persist()
    }

    pub fn attach_turn_bound(
        &mut self,
        identity: &TaskRunIdentity,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<TaskRunIdentity, String> {
        self.require_identity(identity)?;
        self.reject_cancelling()?;
        self.attach_turn(&identity.task_id, thread_id, turn_id)?;
        self.identity_for(&identity.task_id)
            .ok_or("Attached task identity is unavailable".into())
    }

    pub fn update_turn_usage(
        &mut self,
        task_id: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<bool, String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        let input_delta = input_tokens.saturating_sub(task.active_input_tokens);
        let output_delta = output_tokens.saturating_sub(task.active_output_tokens);
        if input_delta == 0 && output_delta == 0 {
            return Ok(caps_exceeded(plan));
        }
        task.active_input_tokens = input_tokens;
        task.active_output_tokens = output_tokens;
        task.input_tokens = task.input_tokens.saturating_add(input_delta);
        task.output_tokens = task.output_tokens.saturating_add(output_delta);
        plan.total_input_tokens = plan.total_input_tokens.saturating_add(input_delta);
        plan.total_output_tokens = plan.total_output_tokens.saturating_add(output_delta);
        let exceeded = caps_exceeded(plan);
        plan.updated_at = now();
        self.persist()?;
        Ok(exceeded)
    }

    pub fn update_turn_usage_bound(
        &mut self,
        identity: &TaskRunIdentity,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<bool, String> {
        self.require_identity(identity)?;
        self.reject_cancelling()?;
        self.update_turn_usage(&identity.task_id, input_tokens, output_tokens)
    }

    pub fn set_human_requests(
        &mut self,
        task_id: &str,
        requests: Vec<HumanRequest>,
    ) -> Result<(), String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if task.status != "running" {
            return Ok(());
        }
        task.human_requests = requests;
        task.status = "needsInput".into();
        task.summary = "Codex is waiting for an explicit human approval or answer".into();
        plan.status = "needsInput".into();
        plan.updated_at = now();
        self.event(
            "approval",
            Some(task_id.into()),
            "Codex App Server request is waiting for a human decision".into(),
        );
        self.persist()
    }

    pub fn set_human_requests_bound(
        &mut self,
        identity: &TaskRunIdentity,
        requests: Vec<HumanRequest>,
    ) -> Result<(), String> {
        self.require_identity(identity)?;
        self.reject_cancelling()?;
        self.set_human_requests(&identity.task_id, requests)
    }

    pub fn pending_request_exact(
        &self,
        plan_id: &str,
        revision: u64,
        task_id: &str,
        request_id: &Value,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(TaskRunIdentity, HumanRequest), String> {
        let identity = TaskRunIdentity {
            plan_id: plan_id.into(),
            revision,
            task_id: task_id.into(),
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
        };
        self.require_identity(&identity)?;
        let plan = self.state.plan.as_ref().ok_or("No autopilot plan")?;
        let task = plan
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if plan.status != "needsInput" || task.status != "needsInput" {
            return Err("This task is no longer waiting for human input".into());
        }
        let request = task
            .human_requests
            .iter()
            .find(|request| {
                &request.request_id == request_id
                    && request.thread_id.as_deref() == Some(thread_id)
                    && request.turn_id.as_deref() == Some(turn_id)
            })
            .cloned()
            .ok_or("The exact pending request no longer exists")?;
        Ok((identity, request))
    }

    pub fn resolve_pending_request(
        &mut self,
        task_id: &str,
        request_id: &Value,
    ) -> Result<(), String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        task.human_requests
            .retain(|request| &request.request_id != request_id);
        if task.human_requests.is_empty() {
            task.status = "running".into();
            task.summary.clear();
            plan.status = if plan.pause_requested {
                "paused"
            } else {
                "running"
            }
            .into();
        }
        plan.updated_at = now();
        self.persist()
    }

    pub fn resolve_pending_request_bound(
        &mut self,
        identity: &TaskRunIdentity,
        request_id: &Value,
    ) -> Result<(), String> {
        self.require_identity(identity)?;
        self.reject_cancelling()?;
        self.resolve_pending_request(&identity.task_id, request_id)
    }

    pub fn record_transport_events(
        &mut self,
        task_id: &str,
        events: Vec<AppServerEvent>,
    ) -> Result<(), String> {
        for event in events {
            match normalize_transport_event(task_id, event) {
                NormalizedTransportEvent::Drop => {}
                NormalizedTransportEvent::Add {
                    kind,
                    message,
                    stream_key,
                } => {
                    let event_id =
                        self.upsert_transport_event(kind, task_id, stream_key.as_ref(), message);
                    if let Some(key) = stream_key {
                        self.track_stream_event(key, event_id);
                    }
                }
                NormalizedTransportEvent::Stream {
                    kind,
                    key,
                    prefix,
                    delta,
                } => {
                    self.append_stream_event(kind, task_id, key, prefix, &delta);
                }
                NormalizedTransportEvent::FinishTurn {
                    kind,
                    message,
                    turn_id,
                } => {
                    self.event(kind, Some(task_id.into()), message);
                    self.stream_events
                        .retain(|key, _| key.task_id != task_id || key.turn_id != turn_id);
                    self.stream_previews
                        .retain(|key, _| key.task_id != task_id || key.turn_id != turn_id);
                }
            }
        }
        self.persist()
    }

    pub fn record_transport_events_bound(
        &mut self,
        identity: &TaskRunIdentity,
        mut events: Vec<AppServerEvent>,
    ) -> Result<(), String> {
        self.require_identity(identity)?;
        self.reject_cancelling()?;
        events.retain(|event| transport_event_matches(identity, event));
        self.record_transport_events(&identity.task_id, events)
    }

    pub fn complete_turn(
        &mut self,
        task_id: &str,
        terminal_status: &str,
        structured_output: Option<&Value>,
        input_tokens: u64,
        output_tokens: u64,
        error: Option<&str>,
    ) -> Result<(), String> {
        let terminal_completed = terminal_status == "completed";
        let outcome = if terminal_completed {
            parse_outcome(
                structured_output.ok_or("Completed turn did not return structured output")?,
            )?
        } else {
            StructuredOutcome::Fail(error.unwrap_or("Codex turn did not complete").into())
        };
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        let index = plan
            .tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if plan.tasks[index].status != "running" {
            return Err("Task is not running".into());
        }
        let input_delta = input_tokens.saturating_sub(plan.tasks[index].active_input_tokens);
        let output_delta = output_tokens.saturating_sub(plan.tasks[index].active_output_tokens);
        plan.tasks[index].input_tokens = plan.tasks[index].input_tokens.saturating_add(input_delta);
        plan.tasks[index].output_tokens =
            plan.tasks[index].output_tokens.saturating_add(output_delta);
        plan.tasks[index].active_input_tokens = input_tokens;
        plan.tasks[index].active_output_tokens = output_tokens;
        plan.total_input_tokens = plan.total_input_tokens.saturating_add(input_delta);
        plan.total_output_tokens = plan.total_output_tokens.saturating_add(output_delta);
        plan.tasks[index].finished_at = Some(now());
        plan.tasks[index].human_requests.clear();
        match outcome {
            StructuredOutcome::Pass(summary) => {
                plan.tasks[index].status = "passed".into();
                plan.tasks[index].summary = summary;
                plan.tasks[index].feedback.clear();
                mark_ready(plan);
                update_terminal_status(plan);
            }
            StructuredOutcome::NeedsInput(summary) => {
                plan.tasks[index].status = "needsInput".into();
                plan.tasks[index].summary = summary;
                plan.status = "needsInput".into();
            }
            StructuredOutcome::Fail(summary) => {
                if terminal_completed && schedule_bounded_fix_loop(plan, index, &summary) {
                    plan.status = if plan.pause_requested {
                        "paused"
                    } else {
                        "running"
                    }
                    .into();
                    mark_ready(plan);
                } else {
                    plan.tasks[index].status = "needsInput".into();
                    let owners = implementation_ancestors(plan, &plan.tasks[index].id);
                    plan.tasks[index].summary = if owners.len() > 1 {
                        format!(
                            "{summary}\nAutomatic repair stopped because this check covers multiple implementation branches ({}). Human direction is required.",
                            owners.join(", ")
                        )
                    } else {
                        summary
                    };
                    plan.status = "needsInput".into();
                }
            }
        }
        enforce_caps(plan);
        plan.updated_at = now();
        self.event(
            "result",
            Some(task_id.into()),
            format!("Task result accepted from terminal turn status: {terminal_status}"),
        );
        self.persist()
    }

    pub fn complete_turn_bound(
        &mut self,
        identity: &TaskRunIdentity,
        terminal_status: &str,
        structured_output: Option<&Value>,
        input_tokens: u64,
        output_tokens: u64,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.require_identity(identity)?;
        self.reject_cancelling()?;
        self.complete_turn(
            &identity.task_id,
            terminal_status,
            structured_output,
            input_tokens,
            output_tokens,
            error,
        )
    }

    pub fn fail_dispatch(&mut self, task_id: &str, message: &str) -> Result<(), String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        if plan.status == "cancelling" {
            return Ok(());
        }
        let task = plan
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or("Unknown task")?;
        if task.status != "running" {
            return Ok(());
        }
        task.status = "needsInput".into();
        task.summary = message.into();
        task.finished_at = Some(now());
        plan.status = "needsInput".into();
        plan.updated_at = now();
        self.event("error", Some(task_id.into()), message.into());
        self.persist()
    }

    pub fn fail_dispatch_bound(
        &mut self,
        identity: &TaskRunIdentity,
        message: &str,
    ) -> Result<(), String> {
        self.require_identity(identity)?;
        self.fail_dispatch(&identity.task_id, message)
    }

    pub fn is_task_running(&self, task_id: &str) -> bool {
        self.state.plan.as_ref().is_some_and(|plan| {
            matches!(plan.status.as_str(), "running" | "paused")
                && plan
                    .tasks
                    .iter()
                    .any(|task| task.id == task_id && task.status == "running")
        })
    }

    pub fn is_task_running_bound(&self, identity: &TaskRunIdentity) -> bool {
        self.require_identity(identity).is_ok() && self.is_task_running(&identity.task_id)
    }

    pub fn task_status_bound(&self, identity: &TaskRunIdentity) -> Result<String, String> {
        self.require_identity(identity)?;
        let plan = self.state.plan.as_ref().ok_or("No autopilot plan")?;
        Ok(plan
            .tasks
            .iter()
            .find(|task| task.id == identity.task_id)
            .ok_or("Unknown task")?
            .status
            .clone())
    }

    pub fn identity_for(&self, task_id: &str) -> Option<TaskRunIdentity> {
        let plan = self.state.plan.as_ref()?;
        let task = plan.tasks.iter().find(|task| task.id == task_id)?;
        Some(TaskRunIdentity {
            plan_id: plan.plan_id.clone(),
            revision: plan.revision,
            task_id: task.id.clone(),
            thread_id: task.thread_id.clone(),
            turn_id: task.turn_id.clone(),
        })
    }

    fn require_identity(&self, identity: &TaskRunIdentity) -> Result<(), String> {
        let current = self
            .identity_for(&identity.task_id)
            .ok_or("Task run no longer exists")?;
        if current != *identity {
            return Err("Task run identity changed; stale update was rejected".into());
        }
        Ok(())
    }

    fn reject_cancelling(&self) -> Result<(), String> {
        if self
            .state
            .plan
            .as_ref()
            .is_some_and(|plan| plan.status == "cancelling")
        {
            return Err("Autopilot is cancelling; late task updates are rejected".into());
        }
        Ok(())
    }

    pub fn remaining_runtime(&self) -> std::time::Duration {
        let Some(plan) = self.state.plan.as_ref() else {
            return std::time::Duration::from_secs(1);
        };
        let limit = plan.limits.max_runtime_minutes.saturating_mul(60);
        let elapsed = plan
            .run_started_at
            .map_or(0, |started| now().saturating_sub(started));
        std::time::Duration::from_secs(limit.saturating_sub(elapsed))
    }

    pub fn interrupt_for_shutdown(&mut self) -> Result<(), String> {
        let Some(plan) = self.state.plan.as_mut() else {
            return Ok(());
        };
        if !matches!(plan.status.as_str(), "running" | "paused" | "needsInput") {
            return Ok(());
        }
        plan.status = "interrupted".into();
        plan.updated_at = now();
        for task in &mut plan.tasks {
            if matches!(task.status.as_str(), "running" | "ready") {
                task.status = "interrupted".into();
                task.finished_at = Some(now());
                task.summary =
                    "Application exited before this task reached a verified terminal result".into();
            }
        }
        self.event(
            "recovery",
            None,
            "Run interrupted during application shutdown".into(),
        );
        self.persist()
    }

    pub fn plan_prompt(&self, task: &PlanTask) -> Result<String, String> {
        let plan = self.state.plan.as_ref().ok_or("No autopilot plan")?;
        let dependency_results = task
            .dependencies
            .iter()
            .filter_map(|id| {
                plan.tasks
                    .iter()
                    .find(|candidate| candidate.id == *id)
                    .map(|dependency| {
                        format!(
                            "- {} ({}): {}",
                            dependency.title, dependency.role, dependency.summary
                        )
                    })
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!(
            "Software Team Autopilot task.\nObjective: {}\nRole: {}\nTask: {}\nInstructions: {}\nDependency results:\n{}\nHuman feedback: {}\nWork only inside: {}\nReturn ONLY the required structured result. outcome=pass only when the assigned work and role checks truly completed; use needs_input when human direction/approval is required; use fail for a verified failure.",
            plan.objective, task.role, task.title, task.description, dependency_results, task.feedback, plan.workspace
        ))
    }

    fn plan_mut(&mut self, plan_id: &str) -> Result<&mut AutopilotPlan, String> {
        let plan = self.state.plan.as_mut().ok_or("No autopilot plan")?;
        if plan.plan_id != plan_id {
            return Err("Unknown plan".into());
        }
        Ok(plan)
    }

    fn event(&mut self, kind: &str, task_id: Option<String>, message: String) {
        self.state.next_event_id = self.state.next_event_id.saturating_add(1);
        self.state.events.push(AutopilotEvent {
            id: self.state.next_event_id,
            at: now(),
            kind: kind.into(),
            task_id,
            message,
        });
        while self.state.events.len() > MAX_EVENTS {
            let removable = self
                .state
                .events
                .iter()
                .position(|event| !is_critical_event(event))
                .unwrap_or(0);
            self.state.events.remove(removable);
        }
    }

    fn upsert_transport_event(
        &mut self,
        kind: &str,
        task_id: &str,
        stream_key: Option<&StreamEventKey>,
        message: String,
    ) -> u64 {
        if let Some(key) = stream_key {
            if let Some(event_id) = self.stream_events.get(key).copied() {
                if let Some(existing) = self
                    .state
                    .events
                    .iter_mut()
                    .find(|event| event.id == event_id)
                {
                    existing.kind = kind.into();
                    existing.message = bounded_event_text(&message);
                    existing.at = now();
                    return event_id;
                }
            }
        }
        self.event(kind, Some(task_id.into()), bounded_event_text(&message));
        self.state.next_event_id
    }

    fn append_stream_event(
        &mut self,
        kind: &str,
        task_id: &str,
        key: StreamEventKey,
        prefix: &str,
        delta: &str,
    ) {
        let preview = self.stream_previews.entry(key.clone()).or_default();
        preview.push_str(delta);
        if preview.chars().count() > MAX_EVENT_MESSAGE_CHARS * 2 {
            *preview = preview.chars().take(MAX_EVENT_MESSAGE_CHARS * 2).collect();
        }
        let clean_preview = compact_whitespace(preview);
        let event_id = self.stream_events.get(&key).copied();
        if let Some(existing) =
            event_id.and_then(|id| self.state.events.iter_mut().find(|event| event.id == id))
        {
            existing.message = bounded_event_text(&format!("{prefix}{clean_preview}"));
            existing.at = now();
            return;
        }
        let message = if clean_preview.is_empty() {
            prefix.trim_end().to_owned()
        } else {
            format!("{prefix}{clean_preview}")
        };
        let id = self.upsert_transport_event(kind, task_id, None, message);
        self.track_stream_event(key, id);
    }

    fn track_stream_event(&mut self, key: StreamEventKey, event_id: u64) {
        if self.stream_events.len() >= MAX_STREAM_TRACKERS && !self.stream_events.contains_key(&key)
        {
            if let Some(oldest) = self
                .stream_events
                .iter()
                .min_by_key(|(_, event_id)| **event_id)
                .map(|(key, _)| key.clone())
            {
                self.stream_events.remove(&oldest);
                self.stream_previews.remove(&oldest);
            }
        }
        self.stream_events.insert(key, event_id);
    }

    fn persist(&mut self) -> Result<(), String> {
        let result = self.persist_inner();
        if let Err(error) = &result {
            self.persistence_blocked = true;
            self.execution_available = false;
            self.connection_message =
                format!("Autopilot persistence failed; execution is blocked: {error}");
            if let Some(plan) = self.state.plan.as_mut() {
                if !matches!(plan.status.as_str(), "completed" | "cancelled" | "failed") {
                    plan.status = "interrupted".into();
                    for task in &mut plan.tasks {
                        if matches!(task.status.as_str(), "running" | "ready") {
                            task.status = "interrupted".into();
                            task.summary = "Persistence failed before this transition was durably checkpointed".into();
                        }
                    }
                }
            }
        }
        result
    }

    fn persist_inner(&self) -> Result<(), String> {
        let parent = self.path.parent().ok_or("Invalid autopilot state path")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        let backup = self.path.with_extension("json.bak");
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if self.path.exists() {
            if backup.exists() {
                fs::remove_file(&backup).map_err(|e| e.to_string())?;
            }
            fs::rename(&self.path, &backup).map_err(|e| e.to_string())?;
        }
        if let Err(error) = fs::rename(&tmp, &self.path) {
            if backup.exists() {
                let _ = fs::rename(&backup, &self.path);
            }
            return Err(error.to_string());
        }
        if backup.exists() {
            fs::remove_file(backup).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn validate_plan_input(input: &AutopilotPlanInput) -> Result<(), String> {
    if input.title.trim().is_empty() || input.objective.trim().is_empty() {
        return Err("Plan title and objective are required".into());
    }
    let workspace = Path::new(&input.workspace);
    if !workspace.is_absolute() || !workspace.is_dir() {
        return Err("Workspace must be an existing absolute directory".into());
    }
    if input.tasks.is_empty() || input.tasks.len() > MAX_TASKS {
        return Err(format!("A plan must contain 1 to {MAX_TASKS} tasks"));
    }
    if !(1..=10).contains(&input.limits.max_attempts) {
        return Err("maxAttempts must be between 1 and 10".into());
    }
    if !(1..=24 * 60).contains(&input.limits.max_runtime_minutes) {
        return Err("maxRuntimeMinutes must be between 1 and 1440".into());
    }
    if input.limits.max_tokens == 0 {
        return Err("maxTokens must be greater than zero".into());
    }
    if input.limits.max_parallel != 1 {
        return Err("V2 requires maxParallel = 1 for a shared workspace".into());
    }
    let allowed: HashSet<&str> = [
        "planner",
        "coder",
        "mobile-dev",
        "tester",
        "reviewer",
        "human",
    ]
    .into_iter()
    .collect();
    let mut ids = HashSet::new();
    for task in &input.tasks {
        if task.id.trim().is_empty()
            || task.title.trim().is_empty()
            || task.description.trim().is_empty()
        {
            return Err("Every task needs an id, title, and description".into());
        }
        if !task
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err(format!(
                "Task id '{}' may contain only letters, numbers, '-' and '_'",
                task.id
            ));
        }
        if !ids.insert(task.id.as_str()) {
            return Err(format!("Duplicate task id: {}", task.id));
        }
        if !allowed.contains(task.role.as_str()) {
            return Err(format!("Unsupported Software Team role: {}", task.role));
        }
    }
    let map: HashMap<&str, &PlanTaskInput> = input
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    for task in &input.tasks {
        for dependency in &task.dependencies {
            if dependency == &task.id || !map.contains_key(dependency.as_str()) {
                return Err(format!(
                    "Task '{}' has an invalid dependency '{}'",
                    task.id, dependency
                ));
            }
        }
    }
    let order = topological_order(&input.tasks)?;
    if order.len() != input.tasks.len() {
        return Err("Dependency map contains a cycle".into());
    }
    let implementations: Vec<_> = input
        .tasks
        .iter()
        .filter(|task| matches!(task.role.as_str(), "coder" | "mobile-dev"))
        .collect();
    if implementations.is_empty() {
        return Err(
            "The Software Team plan needs a Coder or Mobile Dev implementation task".into(),
        );
    }
    for implementation in implementations {
        if !input.tasks.iter().any(|task| {
            task.role == "tester" && depends_transitively(&task.id, &implementation.id, &map)
        }) {
            return Err(format!(
                "Implementation task '{}' must be followed by a Tester task",
                implementation.id
            ));
        }
    }
    for tester in input.tasks.iter().filter(|task| task.role == "tester") {
        if !input
            .tasks
            .iter()
            .any(|task| task.role == "reviewer" && depends_transitively(&task.id, &tester.id, &map))
        {
            return Err(format!(
                "Tester task '{}' must be followed by an independent Reviewer task",
                tester.id
            ));
        }
    }
    let reviewers: Vec<_> = input
        .tasks
        .iter()
        .filter(|task| task.role == "reviewer")
        .collect();
    if reviewers.is_empty() {
        return Err("The plan needs an independent Reviewer task".into());
    }
    for reviewer in reviewers {
        if !input
            .tasks
            .iter()
            .any(|task| task.role == "human" && depends_transitively(&task.id, &reviewer.id, &map))
        {
            return Err(format!(
                "Reviewer task '{}' must be followed by a final Human gate",
                reviewer.id
            ));
        }
    }
    Ok(())
}

fn topological_order(tasks: &[PlanTaskInput]) -> Result<Vec<String>, String> {
    let mut indegree: HashMap<String, usize> = tasks
        .iter()
        .map(|task| (task.id.clone(), task.dependencies.len()))
        .collect();
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in tasks {
        for dependency in &task.dependencies {
            outgoing.entry(dependency).or_default().push(&task.id);
        }
    }
    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut result = Vec::new();
    while let Some(id) = queue.pop_front() {
        result.push(id.clone());
        for dependent in outgoing.get(id.as_str()).into_iter().flatten() {
            let degree = indegree
                .get_mut(*dependent)
                .ok_or("Invalid dependency graph")?;
            *degree -= 1;
            if *degree == 0 {
                queue.push_back((*dependent).into());
            }
        }
    }
    Ok(result)
}

fn depends_transitively<'a>(
    task_id: &str,
    ancestor_id: &str,
    map: &HashMap<&'a str, &'a PlanTaskInput>,
) -> bool {
    let mut stack = vec![task_id];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let Some(task) = map.get(id) else { continue };
        for dependency in &task.dependencies {
            if dependency == ancestor_id {
                return true;
            }
            stack.push(dependency);
        }
    }
    false
}

fn mark_ready(plan: &mut AutopilotPlan) {
    let passed: HashSet<String> = plan
        .tasks
        .iter()
        .filter(|task| task.status == "passed")
        .map(|task| task.id.clone())
        .collect();
    for task in &mut plan.tasks {
        if task.status == "pending"
            && task
                .dependencies
                .iter()
                .all(|dependency| passed.contains(dependency))
        {
            task.status = "ready".into();
        }
    }
}

fn update_terminal_status(plan: &mut AutopilotPlan) {
    if plan.tasks.iter().all(|task| task.status == "passed") {
        plan.status = "completed".into();
    }
}

fn reset_task_and_descendants(plan: &mut AutopilotPlan, task_id: &str, feedback: &str) {
    let affected: HashSet<String> = plan
        .tasks
        .iter()
        .filter(|task| task.id == task_id || runtime_depends_on(plan, &task.id, task_id))
        .map(|task| task.id.clone())
        .collect();
    for task in &mut plan.tasks {
        if affected.contains(&task.id) {
            task.status = "pending".into();
            task.summary.clear();
            task.finished_at = None;
            task.thread_id.clear();
            task.turn_id.clear();
            task.human_requests.clear();
            task.active_input_tokens = 0;
            task.active_output_tokens = 0;
            if task.id == task_id {
                task.feedback = feedback.into();
            }
        }
    }
}

/// A verified implementation/test/review failure is repairable without human
/// direction while both the failing check and its responsible implementation
/// remain inside the approved attempt bound. The implementation and only its
/// downstream checks are invalidated; unrelated completed branches stay valid.
fn schedule_bounded_fix_loop(plan: &mut AutopilotPlan, failed_index: usize, summary: &str) -> bool {
    let failed_role = plan.tasks[failed_index].role.clone();
    let target_id = if matches!(failed_role.as_str(), "coder" | "mobile-dev") {
        Some(plan.tasks[failed_index].id.clone())
    } else if matches!(failed_role.as_str(), "tester" | "reviewer") {
        let ancestors = implementation_ancestors(plan, &plan.tasks[failed_index].id);
        (ancestors.len() == 1).then(|| ancestors[0].clone())
    } else {
        None
    };
    let Some(target_id) = target_id else {
        return false;
    };
    let Some(target_index) = plan.tasks.iter().position(|task| task.id == target_id) else {
        return false;
    };
    if plan.tasks[failed_index].attempts >= plan.limits.max_attempts
        || plan.tasks[target_index].attempts >= plan.limits.max_attempts
    {
        return false;
    }
    reset_task_and_descendants(
        plan,
        &target_id,
        &format!("A downstream {} reported: {}", failed_role, summary),
    );
    true
}

fn implementation_ancestors(plan: &AutopilotPlan, task_id: &str) -> Vec<String> {
    let map: HashMap<&str, &PlanTask> = plan
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let Some(task) = map.get(task_id) else {
        return Vec::new();
    };
    let mut queue: VecDeque<&str> = task.dependencies.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut implementations = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        let Some(candidate) = map.get(id) else {
            continue;
        };
        if matches!(candidate.role.as_str(), "coder" | "mobile-dev") {
            implementations.insert(candidate.id.clone());
            continue;
        }
        queue.extend(candidate.dependencies.iter().map(String::as_str));
    }
    let mut result: Vec<_> = implementations.into_iter().collect();
    result.sort();
    result
}

fn runtime_depends_on(plan: &AutopilotPlan, task_id: &str, ancestor_id: &str) -> bool {
    let map: HashMap<&str, &PlanTask> = plan
        .tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let mut stack = vec![task_id];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(task) = map.get(id) else { continue };
        for dependency in &task.dependencies {
            if dependency == ancestor_id {
                return true;
            }
            stack.push(dependency);
        }
    }
    false
}

fn caps_exceeded(plan: &AutopilotPlan) -> bool {
    let tokens = plan
        .total_input_tokens
        .saturating_add(plan.total_output_tokens);
    let runtime = plan.run_started_at.is_some_and(|started| {
        now().saturating_sub(started) >= plan.limits.max_runtime_minutes.saturating_mul(60)
    });
    tokens >= plan.limits.max_tokens || runtime
}

fn enforce_caps(plan: &mut AutopilotPlan) {
    if caps_exceeded(plan) && !matches!(plan.status.as_str(), "completed" | "failed" | "cancelled")
    {
        plan.status = "needsInput".into();
        for task in &mut plan.tasks {
            if matches!(task.status.as_str(), "pending" | "ready") {
                task.status = "needsInput".into();
                task.summary = "Run limit reached; human direction is required".into();
            }
        }
    }
}

pub fn outcome_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["outcome", "summary"],
        "properties": {
            "outcome": {"type": "string", "enum": ["pass", "needs_input", "fail"]},
            "summary": {"type": "string", "minLength": 1, "maxLength": 4000}
        }
    })
}

pub fn parse_outcome(value: &Value) -> Result<StructuredOutcome, String> {
    let object = value
        .as_object()
        .ok_or("Structured result must be an object")?;
    if object.len() != 2 || !object.contains_key("outcome") || !object.contains_key("summary") {
        return Err("Structured result contains unexpected or missing fields".into());
    }
    let summary = object
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Structured result summary is required")?;
    if summary.chars().count() > 4000 {
        return Err("Structured result summary is too long".into());
    }
    match object.get("outcome").and_then(Value::as_str) {
        Some("pass") => Ok(StructuredOutcome::Pass(summary.into())),
        Some("needs_input") => Ok(StructuredOutcome::NeedsInput(summary.into())),
        Some("fail") => Ok(StructuredOutcome::Fail(summary.into())),
        _ => Err("Structured result outcome is invalid".into()),
    }
}

fn normalize_transport_event(task_id: &str, event: AppServerEvent) -> NormalizedTransportEvent {
    let method = event.method.as_str();
    let params = &event.params;
    let turn_id = transport_turn_id(params);
    let item_id = transport_item_id(params);
    let key = |family: &str| StreamEventKey {
        task_id: task_id.to_owned(),
        turn_id: turn_id.clone(),
        item_id: item_id.clone(),
        family: family.into(),
    };

    match method {
        "thread/tokenUsage/updated" | "turn/diff/updated" => NormalizedTransportEvent::Drop,
        "turn/started" => NormalizedTransportEvent::Add {
            kind: "lifecycle",
            message: "Codex turn started".into(),
            stream_key: None,
        },
        "turn/completed" => {
            let status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("finished");
            let error = params
                .pointer("/turn/error/message")
                .and_then(Value::as_str);
            let (kind, message) = match error {
                Some(error) => (
                    "error",
                    format!("Codex turn {status}: {}", bounded_event_text(error)),
                ),
                None => ("lifecycle", format!("Codex turn {status}")),
            };
            NormalizedTransportEvent::FinishTurn {
                kind,
                message,
                turn_id,
            }
        }
        "item/agentMessage/delta" => NormalizedTransportEvent::Stream {
            kind: "message",
            key: key("message"),
            prefix: "Agent: ",
            delta: string_at(params, &["delta", "text"]).unwrap_or_default(),
        },
        "item/commandExecution/outputDelta" => {
            let count = string_at(params, &["delta", "text"])
                .map(|text| text.chars().count())
                .unwrap_or(0);
            NormalizedTransportEvent::Stream {
                kind: "command",
                key: key("command-output"),
                prefix: "Command produced output: ",
                // Output often contains source, credentials, or huge test logs.
                // Durable history keeps only bounded size counters.
                delta: (count > 0)
                    .then(|| format!("+{count} chars "))
                    .unwrap_or_default(),
            }
        }
        "item/fileChange/outputDelta" => NormalizedTransportEvent::Drop,
        "item/started" | "item/completed" => {
            normalize_item_event(method == "item/completed", params, key)
        }
        "item/commandExecution/requestApproval" => NormalizedTransportEvent::Add {
            kind: "approval",
            message: format!("Command approval required: {}", command_summary(params)),
            stream_key: None,
        },
        "item/fileChange/requestApproval" => NormalizedTransportEvent::Add {
            kind: "approval",
            message: "File change approval required".into(),
            stream_key: None,
        },
        "serverRequest/resolved" => NormalizedTransportEvent::Add {
            kind: "approval",
            message: "Human request resolved".into(),
            stream_key: None,
        },
        "transport/warning" => NormalizedTransportEvent::Add {
            kind: "error",
            message: format!(
                "App Server warning: {}",
                string_at(params, &["message"]).unwrap_or_else(|| "unknown warning".into())
            ),
            stream_key: None,
        },
        "error" => NormalizedTransportEvent::Add {
            kind: "error",
            message: format!(
                "App Server error: {}",
                params
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .or_else(|| params.get("message").and_then(Value::as_str))
                    .map(bounded_event_text)
                    .unwrap_or_else(|| "unknown error".into())
            ),
            stream_key: None,
        },
        _ if method.ends_with("/requestApproval") || method.ends_with("/requestUserInput") => {
            NormalizedTransportEvent::Add {
                kind: "approval",
                message: format!("Human action required: {}", readable_method(method)),
                stream_key: None,
            }
        }
        _ => NormalizedTransportEvent::Drop,
    }
}

fn normalize_item_event<F>(completed: bool, params: &Value, key: F) -> NormalizedTransportEvent
where
    F: Fn(&str) -> StreamEventKey,
{
    let item = params.get("item").unwrap_or(&Value::Null);
    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "commandExecution" => {
            let command = command_summary(item);
            let message = if completed {
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("finished");
                let exit = item
                    .get("exitCode")
                    .and_then(Value::as_i64)
                    .map(|code| format!(", exit {code}"))
                    .unwrap_or_default();
                format!("Command {status}{exit}: {command}")
            } else {
                format!("Running command: {command}")
            };
            NormalizedTransportEvent::Add {
                kind: "command",
                message,
                stream_key: Some(key("command")),
            }
        }
        "fileChange" => {
            let verb = if completed {
                "Changed files"
            } else {
                "Changing files"
            };
            NormalizedTransportEvent::Add {
                kind: "file",
                message: format!("{verb}: {}", file_change_paths(item)),
                stream_key: Some(key("file")),
            }
        }
        "agentMessage" if completed => {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .map(bounded_event_text)
                .unwrap_or_else(|| "message completed".into());
            NormalizedTransportEvent::Add {
                kind: "message",
                message: format!("Agent: {text}"),
                stream_key: Some(key("message")),
            }
        }
        "mcpToolCall" => {
            let server = item.get("server").and_then(Value::as_str).unwrap_or("MCP");
            let tool = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let status = if completed { "finished" } else { "started" };
            NormalizedTransportEvent::Add {
                kind: "tool",
                message: format!("{server} {tool} {status}"),
                stream_key: Some(key("tool")),
            }
        }
        _ => NormalizedTransportEvent::Drop,
    }
}

fn transport_turn_id(params: &Value) -> String {
    params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/turn/id").and_then(Value::as_str))
        .unwrap_or("unknown-turn")
        .to_owned()
}

fn transport_item_id(params: &Value) -> String {
    params
        .get("itemId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/item/id").and_then(Value::as_str))
        .unwrap_or("unknown-item")
        .to_owned()
}

fn transport_event_matches(identity: &TaskRunIdentity, event: &AppServerEvent) -> bool {
    let params = &event.params;
    let turn = params
        .get("turnId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/turn/id").and_then(Value::as_str));
    let thread = params
        .get("threadId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/thread/id").and_then(Value::as_str));
    turn.is_none_or(|turn| turn == identity.turn_id)
        && thread.is_none_or(|thread| thread == identity.thread_id)
}

fn string_at(value: &Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_str))
        .map(str::to_owned)
}

fn command_summary(value: &Value) -> String {
    let command_value = value.get("command").unwrap_or(value);
    let command = command_value
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            command_value.as_array().map(|parts| {
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        })
        .unwrap_or_else(|| "command details unavailable".into());
    bounded_event_text(&command)
}

fn file_change_paths(item: &Value) -> String {
    let paths = item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| change.get("path").and_then(Value::as_str))
        .take(5)
        .map(bounded_event_text)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        "file details unavailable".into()
    } else {
        bounded_event_text(&paths.join(", "))
    }
}

fn readable_method(method: &str) -> String {
    bounded_event_text(&method.replace('/', " "))
}

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bounded_event_text(value: &str) -> String {
    let compact = compact_whitespace(value);
    if compact.chars().count() <= MAX_EVENT_MESSAGE_CHARS {
        return compact;
    }
    let mut result = compact
        .chars()
        .take(MAX_EVENT_MESSAGE_CHARS - 1)
        .collect::<String>();
    result.push('…');
    result
}

fn is_critical_event(event: &AutopilotEvent) -> bool {
    matches!(
        event.kind.as_str(),
        "plan"
            | "approval"
            | "run"
            | "human"
            | "limit"
            | "dispatch"
            | "result"
            | "error"
            | "recovery"
            | "lifecycle"
    ) || (event.kind == "command"
        && (event.message.starts_with("Command completed")
            || event.message.starts_with("Command failed")
            || event.message.contains(", exit ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);

    fn workspace() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "acc-autopilot-{}-{}-{}",
            std::process::id(),
            now(),
            NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn valid_input(root: &Path) -> AutopilotPlanInput {
        AutopilotPlanInput {
            plan_id: None,
            title: "Ship change".into(),
            objective: "Implement safely".into(),
            workspace: root.to_string_lossy().into(),
            limits: AutopilotLimits {
                max_attempts: 2,
                max_runtime_minutes: 60,
                max_tokens: 10_000,
                max_parallel: 1,
            },
            tasks: vec![
                PlanTaskInput {
                    id: "code".into(),
                    title: "Code".into(),
                    description: "Implement".into(),
                    role: "coder".into(),
                    dependencies: vec![],
                    model: None,
                    effort: None,
                },
                PlanTaskInput {
                    id: "test".into(),
                    title: "Test".into(),
                    description: "Verify".into(),
                    role: "tester".into(),
                    dependencies: vec!["code".into()],
                    model: None,
                    effort: None,
                },
                PlanTaskInput {
                    id: "review".into(),
                    title: "Review".into(),
                    description: "Review independently".into(),
                    role: "reviewer".into(),
                    dependencies: vec!["test".into()],
                    model: None,
                    effort: None,
                },
                PlanTaskInput {
                    id: "human".into(),
                    title: "Approve".into(),
                    description: "Final human review".into(),
                    role: "human".into(),
                    dependencies: vec!["review".into()],
                    model: None,
                    effort: None,
                },
            ],
        }
    }

    #[test]
    fn validation_rejects_cycles_and_missing_quality_chain() {
        let root = workspace();
        let mut cycle = valid_input(&root);
        cycle.tasks[0].dependencies.push("review".into());
        assert!(validate_plan_input(&cycle).unwrap_err().contains("cycle"));
        let mut missing = valid_input(&root);
        missing.tasks.retain(|task| task.role != "reviewer");
        missing.tasks.last_mut().unwrap().dependencies = vec!["test".into()];
        assert!(validate_plan_input(&missing).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn edits_invalidate_approval_and_start_requires_exact_revision() {
        let root = workspace();
        let path = root.join("state.json");
        let mut controller = AutopilotController::new(path).unwrap();
        let snapshot = controller.save_plan(valid_input(&root)).unwrap();
        let plan = snapshot.plan.unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        assert!(controller.start(&plan.plan_id, plan.revision + 1).is_err());
        controller.start(&plan.plan_id, plan.revision).unwrap();
        assert_eq!(controller.next_ready_task().unwrap().id, "code");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_terminal_structured_results_advance_dependencies() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        controller
            .mark_turn_started("code", "thread", "turn")
            .unwrap();
        assert!(controller
            .complete_turn(
                "code",
                "running",
                Some(&json!({"outcome":"pass","summary":"done"})),
                1,
                1,
                None
            )
            .is_ok());
        assert_eq!(
            controller.snapshot().plan.unwrap().tasks[0].status,
            "needsInput"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_chain_waits_at_final_human_gate() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        for id in ["code", "test", "review"] {
            controller
                .mark_turn_started(id, &format!("thread-{id}"), &format!("turn-{id}"))
                .unwrap();
            controller
                .complete_turn(
                    id,
                    "completed",
                    Some(&json!({"outcome":"pass","summary":format!("{id} passed")})),
                    10,
                    2,
                    None,
                )
                .unwrap();
        }
        assert!(controller.activate_human_gate().unwrap());
        assert_eq!(
            controller.snapshot().plan.as_ref().unwrap().status,
            "needsInput"
        );
        controller
            .respond_gate(&plan.plan_id, "human", "Looks good", true, false)
            .unwrap();
        assert_eq!(controller.snapshot().plan.unwrap().status, "completed");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restart_never_replays_running_work() {
        let root = workspace();
        let path = root.join("state.json");
        let mut controller = AutopilotController::new(path.clone()).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        controller
            .mark_turn_started("code", "thread", "turn")
            .unwrap();
        drop(controller);
        let recovered = AutopilotController::new(path)
            .unwrap()
            .snapshot()
            .plan
            .unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.tasks[0].status, "needsInput");
        assert!(recovered.tasks[0].human_requests.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structured_outcome_rejects_free_text_and_extra_fields() {
        assert!(parse_outcome(&json!("pass")).is_err());
        assert!(parse_outcome(&json!({"outcome":"pass","summary":"ok","claim":true})).is_err());
        assert_eq!(
            parse_outcome(&json!({"outcome":"pass","summary":"ok"})).unwrap(),
            StructuredOutcome::Pass("ok".into())
        );
    }

    #[test]
    fn tester_failure_reopens_its_single_implementation_and_stops_at_attempt_cap() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        for round in 0..2 {
            controller
                .mark_turn_started("code", "code-thread", "code-turn")
                .unwrap();
            controller
                .complete_turn(
                    "code",
                    "completed",
                    Some(&json!({"outcome":"pass","summary":"implemented"})),
                    1,
                    1,
                    None,
                )
                .unwrap();
            controller
                .mark_turn_started("test", "test-thread", "test-turn")
                .unwrap();
            controller
                .complete_turn(
                    "test",
                    "completed",
                    Some(&json!({"outcome":"fail","summary":"regression"})),
                    1,
                    1,
                    None,
                )
                .unwrap();
            let snapshot = controller.snapshot().plan.unwrap();
            if round == 0 {
                assert_eq!(snapshot.tasks[0].status, "ready");
                assert_eq!(snapshot.tasks[1].status, "pending");
            } else {
                assert_eq!(snapshot.status, "needsInput");
                assert_eq!(snapshot.tasks[1].status, "needsInput");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn joined_branches_require_human_direction_instead_of_guessing_repair_owner() {
        let root = workspace();
        let mut input = valid_input(&root);
        input.tasks.insert(
            1,
            PlanTaskInput {
                id: "mobile".into(),
                title: "Mobile".into(),
                description: "Implement mobile".into(),
                role: "mobile-dev".into(),
                dependencies: vec![],
                model: None,
                effort: None,
            },
        );
        input
            .tasks
            .iter_mut()
            .find(|task| task.id == "test")
            .unwrap()
            .dependencies
            .push("mobile".into());
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller.save_plan(input).unwrap().plan.unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        for id in ["code", "mobile"] {
            controller.mark_turn_started(id, "thread", "turn").unwrap();
            controller
                .complete_turn(
                    id,
                    "completed",
                    Some(&json!({"outcome":"pass","summary":"done"})),
                    1,
                    1,
                    None,
                )
                .unwrap();
        }
        controller
            .mark_turn_started("test", "test-thread", "test-turn")
            .unwrap();
        controller
            .complete_turn(
                "test",
                "completed",
                Some(&json!({"outcome":"fail","summary":"integration failure"})),
                1,
                1,
                None,
            )
            .unwrap();
        let plan = controller.snapshot().plan.unwrap();
        let test = plan.tasks.iter().find(|task| task.id == "test").unwrap();
        assert_eq!(plan.status, "needsInput");
        assert!(test.summary.contains("multiple implementation branches"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persistence_failure_latches_execution_off() {
        let root = workspace();
        let workspace_root = root.join("workspace");
        fs::create_dir_all(&workspace_root).unwrap();
        let state_parent = root.join("state-parent");
        fs::create_dir_all(&state_parent).unwrap();
        let mut controller = AutopilotController::new(state_parent.join("state.json")).unwrap();
        fs::remove_dir(&state_parent).unwrap();
        fs::write(&state_parent, b"blocks directory recreation").unwrap();
        let error = controller
            .save_plan(valid_input(&workspace_root))
            .unwrap_err();
        assert!(!error.is_empty());
        let snapshot = controller.snapshot();
        assert!(!snapshot.execution_available);
        assert_eq!(snapshot.plan.unwrap().status, "interrupted");
        fs::remove_file(state_parent).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_protocol_request_pauses_and_resumes_the_same_turn() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        controller.reserve_task("code").unwrap();
        controller
            .attach_turn("code", "thread-1", "turn-1")
            .unwrap();
        let request = HumanRequest {
            request_id: json!(7),
            method: "item/commandExecution/requestApproval".into(),
            thread_id: Some("thread-1".into()),
            turn_id: Some("turn-1".into()),
            params: json!({"command":"cargo test"}),
        };
        controller
            .set_human_requests("code", vec![request])
            .unwrap();
        assert_eq!(
            controller.snapshot().plan.as_ref().unwrap().status,
            "needsInput"
        );
        controller
            .resolve_pending_request("code", &json!(7))
            .unwrap();
        let mut resumed = controller.snapshot().plan.unwrap();
        let task = resumed.tasks.remove(0);
        assert_eq!(task.status, "running");
        assert_eq!(task.turn_id, "turn-1");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retrying_changed_implementation_invalidates_downstream_evidence() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        let plan_ref = controller.state.plan.as_mut().unwrap();
        for task in &mut plan_ref.tasks {
            task.status = "passed".into();
        }
        plan_ref.tasks[0].status = "needsInput".into();
        plan_ref.status = "needsInput".into();
        controller
            .respond_gate(&plan.plan_id, "code", "apply requested fix", false, true)
            .unwrap();
        let plan = controller.snapshot().plan.unwrap();
        assert_eq!(plan.tasks[0].status, "ready");
        assert!(plan
            .tasks
            .iter()
            .skip(1)
            .all(|task| task.status == "pending"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_turn_identity_cannot_update_a_new_revision_with_reused_task_ids() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let mut input = valid_input(&root);
        let first = controller.save_plan(input.clone()).unwrap().plan.unwrap();
        controller
            .approve_plan(&first.plan_id, first.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&first.plan_id, first.revision).unwrap();
        let reserved = controller.reserve_task_bound("code").unwrap();
        let stale = controller
            .attach_turn_bound(&reserved, "old-thread", "old-turn")
            .unwrap();
        controller.begin_cancel(&first.plan_id).unwrap();
        controller.finish_cancel(&first.plan_id).unwrap();
        input.plan_id = Some(first.plan_id.clone());
        let second = controller.save_plan(input).unwrap().plan.unwrap();
        controller
            .approve_plan(&second.plan_id, second.revision)
            .unwrap();
        controller.start(&second.plan_id, second.revision).unwrap();
        let reserved = controller.reserve_task_bound("code").unwrap();
        let current = controller
            .attach_turn_bound(&reserved, "new-thread", "new-turn")
            .unwrap();
        assert!(controller
            .complete_turn_bound(
                &stale,
                "completed",
                Some(&json!({"outcome":"pass","summary":"stale"})),
                1,
                1,
                None
            )
            .is_err());
        assert!(controller.is_task_running_bound(&current));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_pending_request_identity_rejects_shifted_or_wrong_turn() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        let reserved = controller.reserve_task_bound("code").unwrap();
        let identity = controller
            .attach_turn_bound(&reserved, "thread", "turn")
            .unwrap();
        let requests = [1, 2]
            .into_iter()
            .map(|id| HumanRequest {
                request_id: json!(id),
                method: "item/commandExecution/requestApproval".into(),
                thread_id: Some("thread".into()),
                turn_id: Some("turn".into()),
                params: json!({"id":id}),
            })
            .collect();
        controller
            .set_human_requests_bound(&identity, requests)
            .unwrap();
        assert!(controller
            .pending_request_exact(
                &plan.plan_id,
                plan.revision,
                "code",
                &json!(2),
                "thread",
                "wrong-turn"
            )
            .is_err());
        let (_, exact) = controller
            .pending_request_exact(
                &plan.plan_id,
                plan.revision,
                "code",
                &json!(2),
                "thread",
                "turn",
            )
            .unwrap();
        assert_eq!(exact.request_id, json!(2));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovered_pending_request_ids_are_invalidated() {
        let root = workspace();
        let path = root.join("state.json");
        let mut controller = AutopilotController::new(path.clone()).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        let reserved = controller.reserve_task_bound("code").unwrap();
        let identity = controller
            .attach_turn_bound(&reserved, "thread", "turn")
            .unwrap();
        controller
            .set_human_requests_bound(
                &identity,
                vec![HumanRequest {
                    request_id: json!(9),
                    method: "item/commandExecution/requestApproval".into(),
                    thread_id: Some("thread".into()),
                    turn_id: Some("turn".into()),
                    params: json!({}),
                }],
            )
            .unwrap();
        drop(controller);
        let recovered = AutopilotController::new(path)
            .unwrap()
            .snapshot()
            .plan
            .unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.tasks[0].status, "needsInput");
        assert!(recovered.tasks[0].human_requests.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn paused_failure_prepares_retry_without_resuming_dispatch() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        controller
            .mark_turn_started("code", "thread", "turn")
            .unwrap();
        controller.pause(&plan.plan_id).unwrap();
        controller
            .complete_turn(
                "code",
                "completed",
                Some(&json!({"outcome":"fail","summary":"fix me"})),
                1,
                1,
                None,
            )
            .unwrap();
        let paused = controller.snapshot().plan.unwrap();
        assert_eq!(paused.status, "paused");
        assert_eq!(paused.tasks[0].status, "ready");
        assert!(controller.next_ready_task().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelling_blocks_plan_edits_until_finish() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let input = valid_input(&root);
        let plan = controller.save_plan(input.clone()).unwrap().plan.unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        controller.begin_cancel(&plan.plan_id).unwrap();
        assert_eq!(
            controller.snapshot().plan.as_ref().unwrap().status,
            "cancelling"
        );
        let mut edit = input;
        edit.plan_id = Some(plan.plan_id.clone());
        assert!(controller.save_plan(edit).is_err());
        controller.finish_cancel(&plan.plan_id).unwrap();
        assert_eq!(controller.snapshot().plan.unwrap().status, "cancelled");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelling_rejects_gate_retry_and_remains_finishable() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        controller
            .mark_turn_started("code", "thread", "turn")
            .unwrap();
        controller
            .complete_turn(
                "code",
                "completed",
                Some(&json!({"outcome":"needs_input","summary":"direction"})),
                1,
                1,
                None,
            )
            .unwrap();
        controller.begin_cancel(&plan.plan_id).unwrap();
        assert!(controller
            .respond_gate(&plan.plan_id, "code", "retry", false, true)
            .is_err());
        assert_eq!(
            controller.snapshot().plan.as_ref().unwrap().status,
            "cancelling"
        );
        controller.finish_cancel(&plan.plan_id).unwrap();
        assert_eq!(controller.snapshot().plan.unwrap().status, "cancelled");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dispatch_failure_cannot_clobber_cancelling_state() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        let reserved = controller.reserve_task_bound("code").unwrap();
        controller.begin_cancel(&plan.plan_id).unwrap();
        controller
            .fail_dispatch_bound(&reserved, "late dispatch error")
            .unwrap();
        assert_eq!(
            controller.snapshot().plan.as_ref().unwrap().status,
            "cancelling"
        );
        controller.finish_cancel(&plan.plan_id).unwrap();
        assert_eq!(controller.snapshot().plan.unwrap().status, "cancelled");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelling_rejects_other_bound_turn_mutations() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let plan = controller
            .save_plan(valid_input(&root))
            .unwrap()
            .plan
            .unwrap();
        controller
            .approve_plan(&plan.plan_id, plan.revision)
            .unwrap();
        controller.set_connection(true, "ready");
        controller.start(&plan.plan_id, plan.revision).unwrap();
        let reserved = controller.reserve_task_bound("code").unwrap();
        let identity = controller
            .attach_turn_bound(&reserved, "thread", "turn")
            .unwrap();
        controller.begin_cancel(&plan.plan_id).unwrap();
        assert!(controller
            .update_turn_usage_bound(&identity, 10, 2)
            .is_err());
        assert!(controller
            .set_human_requests_bound(&identity, vec![])
            .is_err());
        assert!(controller
            .record_transport_events_bound(
                &identity,
                vec![AppServerEvent {
                    method: "turn/started".into(),
                    params: json!({"turnId":"turn"}),
                }],
            )
            .is_err());
        assert!(controller
            .complete_turn_bound(
                &identity,
                "completed",
                Some(&json!({"outcome":"pass","summary":"late"})),
                10,
                2,
                None
            )
            .is_err());
        let cancelling = controller.snapshot().plan.unwrap();
        assert_eq!(cancelling.status, "cancelling");
        assert_eq!(cancelling.total_input_tokens, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transport_delta_flood_coalesces_and_keeps_terminal_and_human_events() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let mut events = (0..1_000)
            .map(|index| AppServerEvent {
                method: "item/agentMessage/delta".into(),
                params: json!({
                    "threadId":"thread-a",
                    "turnId":"turn-a",
                    "itemId":"message-a",
                    "delta": if index % 3 == 0 { "Xin " } else if index % 3 == 1 { "chào " } else { "🌍 " }
                }),
            })
            .collect::<Vec<_>>();
        events.push(AppServerEvent {
            method: "item/commandExecution/requestApproval".into(),
            params: json!({"threadId":"thread-a","turnId":"turn-a","command":"npm test"}),
        });
        events.push(AppServerEvent {
            method: "turn/completed".into(),
            params: json!({"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}),
        });
        controller.record_transport_events("code", events).unwrap();

        let events = controller.snapshot().events;
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "message")
                .count(),
            1
        );
        assert!(events
            .iter()
            .all(|event| event.message.chars().count() <= MAX_EVENT_MESSAGE_CHARS));
        assert!(events
            .iter()
            .any(|event| event.kind == "approval" && event.message.contains("npm test")));
        assert!(events
            .iter()
            .any(|event| event.kind == "lifecycle" && event.message.contains("completed")));
        assert!(events
            .iter()
            .any(|event| event.message.contains("Xin chào 🌍")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transport_scope_and_stream_keys_isolate_turns() {
        let identity = TaskRunIdentity {
            plan_id: "plan".into(),
            revision: 1,
            task_id: "code".into(),
            thread_id: "thread-a".into(),
            turn_id: "turn-a".into(),
        };
        assert!(transport_event_matches(
            &identity,
            &AppServerEvent {
                method: "turn/started".into(),
                params: json!({"threadId":"thread-a","turn":{"id":"turn-a"}}),
            }
        ));
        assert!(!transport_event_matches(
            &identity,
            &AppServerEvent {
                method: "turn/started".into(),
                params: json!({"threadId":"thread-a","turn":{"id":"turn-b"}}),
            }
        ));

        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        controller
            .record_transport_events(
                "code",
                vec![
                    AppServerEvent {
                        method: "item/agentMessage/delta".into(),
                        params: json!({"turnId":"turn-a","itemId":"same","delta":"first"}),
                    },
                    AppServerEvent {
                        method: "item/agentMessage/delta".into(),
                        params: json!({"turnId":"turn-b","itemId":"same","delta":"second"}),
                    },
                ],
            )
            .unwrap();
        let messages = controller
            .snapshot()
            .events
            .into_iter()
            .filter(|event| event.kind == "message")
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().any(|event| event.message.contains("first")));
        assert!(messages
            .iter()
            .any(|event| event.message.contains("second")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_output_is_bounded_and_raw_output_is_not_persisted() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        controller.record_transport_events("test", vec![
            AppServerEvent { method: "item/started".into(), params: json!({"turnId":"turn","item":{"id":"cmd","type":"commandExecution","command":"npm test"}}) },
            AppServerEvent { method: "item/commandExecution/outputDelta".into(), params: json!({"turnId":"turn","itemId":"cmd","delta":"SECRET_TOKEN=should-not-be-in-history".repeat(1000)}) },
            AppServerEvent { method: "item/completed".into(), params: json!({"turnId":"turn","item":{"id":"cmd","type":"commandExecution","command":"npm test","status":"completed","exitCode":0}}) },
        ]).unwrap();
        let events = controller.snapshot().events;
        assert!(events
            .iter()
            .any(|event| event.message.contains("exit 0") && event.message.contains("npm test")));
        assert!(events.iter().any(|event| event.message.contains("chars")));
        assert!(events
            .iter()
            .all(|event| !event.message.contains("SECRET_TOKEN")));
        assert!(events
            .iter()
            .all(|event| event.message.chars().count() <= MAX_EVENT_MESSAGE_CHARS));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_history_prefers_lifecycle_over_unique_stream_noise() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let mut events = vec![AppServerEvent {
            method: "item/fileChange/requestApproval".into(),
            params: json!({"turnId":"turn"}),
        }];
        events.extend((0..700).map(|index| AppServerEvent {
            method: "item/agentMessage/delta".into(),
            params: json!({"turnId":"turn","itemId":format!("message-{index}"),"delta":"noise"}),
        }));
        events.push(AppServerEvent {
            method: "turn/completed".into(),
            params: json!({"turn":{"id":"turn","status":"completed"}}),
        });
        controller.record_transport_events("code", events).unwrap();

        let retained = controller.snapshot().events;
        assert_eq!(retained.len(), MAX_EVENTS);
        assert!(retained.iter().any(|event| event.kind == "approval"));
        assert!(retained.iter().any(|event| event.kind == "lifecycle"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_history_preserves_final_command_result_during_stream_noise() {
        let root = workspace();
        let mut controller = AutopilotController::new(root.join("state.json")).unwrap();
        let mut events = vec![AppServerEvent {
            method: "item/completed".into(),
            params: json!({"turnId":"turn","item":{"id":"cmd","type":"commandExecution","command":"npm test","status":"completed","exitCode":0}}),
        }];
        events.extend((0..700).map(|index| AppServerEvent {
            method: "item/agentMessage/delta".into(),
            params: json!({"turnId":"turn","itemId":format!("message-{index}"),"delta":"noise"}),
        }));
        controller.record_transport_events("test", events).unwrap();

        let retained = controller.snapshot().events;
        assert_eq!(retained.len(), MAX_EVENTS);
        assert!(retained.iter().any(|event| {
            event.kind == "command"
                && event.message.contains("exit 0")
                && event.message.contains("npm test")
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
