//! Small, dependency-free client for the local Codex app-server JSONL protocol.
//!
//! It deliberately does not resolve server-initiated approval or input requests.
//! Those requests are surfaced to the controller as human gates.  This prevents
//! an unattended scheduler from silently granting filesystem, command, network,
//! or MCP permissions.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_EVENT_BACKLOG: usize = 2_000;
const MAX_COALESCED_DELTA_CHARS: usize = 640;

#[derive(Clone, Debug)]
pub struct ThreadStartParams {
    pub cwd: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// One of planner, coder, mobile-dev, tester, or reviewer.
    pub role: String,
}

#[derive(Clone, Debug)]
pub struct TurnStartParams {
    pub thread_id: String,
    pub prompt: String,
    pub output_schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartResult {
    pub thread_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResult {
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HumanRequest {
    pub request_id: Value,
    pub method: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub params: Value,
}

/// A decision produced by an explicit human action in the UI. The transport
/// never constructs one of these on its own.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum HumanDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
    Submit { value: Value },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedTurnResult {
    pub thread_id: String,
    pub turn_id: String,
    pub status: String,
    pub structured_output: Option<Value>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub error: Option<String>,
    pub human_requests: Vec<HumanRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnProgress {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub terminal: bool,
    pub status: Option<String>,
    pub human_requests: Vec<HumanRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerEvent {
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapability {
    pub id: String,
    pub display_name: String,
    pub default_effort: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProbe {
    pub platform_family: String,
    pub platform_os: String,
    pub models: Vec<ModelCapability>,
}

#[derive(Default)]
struct TurnState {
    thread_id: String,
    status: Option<String>,
    structured_output: Option<Value>,
    input_tokens: u64,
    output_tokens: u64,
    error: Option<String>,
    human_requests: Vec<HumanRequest>,
}

#[derive(Clone)]
struct ThreadPolicy {
    cwd: String,
    writable: bool,
}

struct SharedState {
    dead: Option<String>,
    events: VecDeque<AppServerEvent>,
    turns: HashMap<String, TurnState>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            dead: None,
            events: VecDeque::new(),
            turns: HashMap::new(),
        }
    }
}

struct Inner {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Child>>,
    pending: Mutex<HashMap<u64, SyncSender<Result<Value, String>>>>,
    thread_policies: Mutex<HashMap<String, ThreadPolicy>>,
    state: Mutex<SharedState>,
    changed: Condvar,
    next_id: AtomicU64,
    handles: AtomicUsize,
    shutting_down: AtomicBool,
    timeout: Duration,
    initialize_result: Mutex<Value>,
}

/// Cloneable so a scheduler can wait on one clone and interrupt on another.
pub struct AppServerClient {
    inner: Arc<Inner>,
}

impl AppServerClient {
    pub fn spawn(codex_bin: impl AsRef<Path>, timeout: Duration) -> Result<Self, String> {
        if timeout.is_zero() {
            return Err("app-server timeout must be greater than zero".into());
        }
        let codex_bin = resolve_codex_binary(codex_bin.as_ref())?;
        let mut command = Command::new(&codex_bin);
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_windows_console(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start Codex app-server: {e}"))?;
        let stdin = child.stdin.take().ok_or("Codex app-server has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Codex app-server has no stdout")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Codex app-server has no stderr")?;

        let inner = Arc::new(Inner {
            writer: Mutex::new(Some(Box::new(stdin))),
            child: Mutex::new(Some(child)),
            pending: Mutex::new(HashMap::new()),
            thread_policies: Mutex::new(HashMap::new()),
            state: Mutex::new(SharedState::default()),
            changed: Condvar::new(),
            next_id: AtomicU64::new(1),
            handles: AtomicUsize::new(1),
            shutting_down: AtomicBool::new(false),
            timeout,
            initialize_result: Mutex::new(Value::Null),
        });
        spawn_stdout_reader(Arc::clone(&inner), stdout);
        // Drain stderr so a noisy child cannot deadlock. Its contents are not
        // retained because diagnostics can contain machine-specific data.
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut sink = [0_u8; 4096];
            while reader.read(&mut sink).ok().filter(|n| *n > 0).is_some() {}
        });

        let client = Self { inner };
        let initialized = client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "agent_control_center",
                    "title": "Agent Control Center",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            timeout,
        )?;
        *client.inner.initialize_result.lock().map_err(lock_error)? = initialized;
        client.notify("initialized", json!({}))?;
        Ok(client)
    }

    /// Performs only initialize + model/list. It never starts a model turn.
    pub fn capability_probe(&self) -> Result<CapabilityProbe, String> {
        let response = self.request(
            "model/list",
            json!({"limit": 100, "includeHidden": false}),
            self.inner.timeout,
        )?;
        let models = response
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| {
                Some(ModelCapability {
                    id: model.get("id")?.as_str()?.to_owned(),
                    display_name: model
                        .get("displayName")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| model.get("id").and_then(Value::as_str).unwrap_or(""))
                        .to_owned(),
                    default_effort: model
                        .get("defaultReasoningEffort")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    efforts: model
                        .get("supportedReasoningEfforts")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|effort| {
                            effort
                                .get("reasoningEffort")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect(),
                })
            })
            .collect();
        let initialized = self.inner.initialize_result.lock().map_err(lock_error)?;
        Ok(CapabilityProbe {
            platform_family: initialized
                .get("platformFamily")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            platform_os: initialized
                .get("platformOs")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            models,
        })
    }

    pub fn start_thread(&self, params: ThreadStartParams) -> Result<ThreadStartResult, String> {
        let cwd = checked_cwd(&params.cwd)?;
        let writable = matches!(params.role.as_str(), "coder" | "mobile-dev" | "tester");
        let mut request = json!({
            "cwd": cwd.clone(),
            "approvalPolicy": "on-request",
            "sandbox": if writable { "workspace-write" } else { "read-only" },
            "ephemeral": false
        });
        if let Some(model) = nonempty(params.model) {
            request["model"] = Value::String(model);
        }
        if let Some(effort) = nonempty(params.effort) {
            request["config"] = json!({"model_reasoning_effort": effort});
        }
        let result = self.request("thread/start", request, self.inner.timeout)?;
        let thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Codex thread/start returned no thread id")?
            .to_owned();
        self.inner
            .thread_policies
            .lock()
            .map_err(lock_error)?
            .insert(
                thread_id.clone(),
                ThreadPolicy {
                    cwd: cwd.to_owned(),
                    writable,
                },
            );
        Ok(ThreadStartResult { thread_id })
    }

    pub fn start_turn(&self, params: TurnStartParams) -> Result<TurnStartResult, String> {
        if params.thread_id.trim().is_empty() || params.prompt.trim().is_empty() {
            return Err("thread id and prompt are required".into());
        }
        if !params.output_schema.is_object() {
            return Err("output schema must be a JSON object".into());
        }
        let policy = self
            .inner
            .thread_policies
            .lock()
            .map_err(lock_error)?
            .get(&params.thread_id)
            .cloned()
            .ok_or("No managed sandbox policy is registered for this Codex thread")?;
        let sandbox_policy = sandbox_policy(&policy);
        let result = self.request(
            "turn/start",
            json!({
                "threadId": params.thread_id.clone(),
                "input": [{"type": "text", "text": params.prompt}],
                "outputSchema": params.output_schema,
                "sandboxPolicy": sandbox_policy,
                "approvalPolicy": "on-request"
            }),
            self.inner.timeout,
        )?;
        let turn_id = result
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Codex turn/start returned no turn id")?
            .to_owned();
        let mut state = self.inner.state.lock().map_err(lock_error)?;
        let turn = state.turns.entry(turn_id.clone()).or_default();
        if turn.thread_id.is_empty() {
            turn.thread_id = params.thread_id.clone();
        }
        Ok(TurnStartResult {
            thread_id: params.thread_id,
            turn_id,
        })
    }

    pub fn wait_turn(&self, turn_id: &str, timeout: Duration) -> Result<ManagedTurnResult, String> {
        if timeout.is_zero() {
            return Err("turn wait timeout must be greater than zero".into());
        }
        let deadline = Instant::now() + timeout;
        let mut state = self.inner.state.lock().map_err(lock_error)?;
        loop {
            if let Some(dead) = &state.dead {
                return Err(dead.clone());
            }
            if let Some(turn) = state.turns.get(turn_id) {
                if turn.status.is_some() {
                    return Ok(turn_result(
                        turn_id,
                        turn,
                        turn.status.as_deref().unwrap_or("failed"),
                    ));
                }
                if !turn.human_requests.is_empty() {
                    return Ok(turn_result(turn_id, turn, "needs_input"));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Timed out waiting for Codex turn {turn_id}"));
            }
            let (next, wait) = self
                .inner
                .changed
                .wait_timeout(state, remaining)
                .map_err(lock_error)?;
            state = next;
            if wait.timed_out() {
                return Err(format!("Timed out waiting for Codex turn {turn_id}"));
            }
        }
    }

    /// Returns the latest in-memory state without waiting or performing I/O.
    /// The scheduler uses this for responsive pause/cancel and budget checks.
    pub fn turn_progress(&self, turn_id: &str) -> Result<TurnProgress, String> {
        let state = self.inner.state.lock().map_err(lock_error)?;
        if let Some(error) = &state.dead {
            return Err(error.clone());
        }
        let turn = state
            .turns
            .get(turn_id)
            .ok_or_else(|| format!("Unknown managed Codex turn {turn_id}"))?;
        Ok(TurnProgress {
            input_tokens: turn.input_tokens,
            output_tokens: turn.output_tokens,
            terminal: turn.status.is_some(),
            status: turn.status.clone(),
            human_requests: turn.human_requests.clone(),
        })
    }

    pub fn interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        self.request(
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
            self.inner.timeout,
        )?;
        Ok(())
    }

    /// Answers a pending server request after an explicit human decision.
    /// Permission accepts grant exactly the requested subset for this turn;
    /// arbitrary expansions and automatic session grants are not supported.
    pub fn respond_to_human_request(
        &self,
        request: &HumanRequest,
        decision: HumanDecision,
    ) -> Result<(), String> {
        let result = human_response_payload(request, decision)?;
        // Keep notification processing excluded across validation and write so
        // a terminal/resolved event cannot make this response stale in-between.
        let mut state = self.inner.state.lock().map_err(lock_error)?;
        if let Some(error) = &state.dead {
            return Err(error.clone());
        }
        take_pending_human_request(&mut state, request)?;
        if let Err(error) =
            self.write_message_unchecked(&json!({"id": request.request_id, "result": result}))
        {
            attach_human_request(&mut state, request.clone());
            return Err(error);
        }
        drop(state);
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn drain_events(&self) -> Vec<AppServerEvent> {
        match self.inner.state.lock() {
            Ok(mut state) => state.events.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn shutdown(&self) {
        if self.inner.shutting_down.swap(true, Ordering::SeqCst) {
            return;
        }
        mark_dead(&self.inner, "Codex app-server was shut down".into());
        if let Ok(mut writer) = self.inner.writer.lock() {
            writer.take();
        }
        if let Ok(mut child_slot) = self.inner.child.lock() {
            if let Some(child) = child_slot.as_mut() {
                let exited = child.try_wait().ok().flatten().is_some();
                if !exited {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            child_slot.take();
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write_message(&json!({"method": method, "params": params}))
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::sync_channel(1);
        self.inner
            .pending
            .lock()
            .map_err(lock_error)?
            .insert(id, sender);
        if let Err(error) =
            self.write_message(&json!({"method": method, "id": id, "params": params}))
        {
            let _ = self
                .inner
                .pending
                .lock()
                .map(|mut pending| pending.remove(&id));
            return Err(error);
        }
        receive_response(receiver, timeout, method).map_err(|error| {
            let _ = self
                .inner
                .pending
                .lock()
                .map(|mut pending| pending.remove(&id));
            error
        })
    }

    fn write_message(&self, message: &Value) -> Result<(), String> {
        if let Some(error) = self.inner.state.lock().map_err(lock_error)?.dead.clone() {
            return Err(error);
        }
        self.write_message_unchecked(message)
    }

    fn write_message_unchecked(&self, message: &Value) -> Result<(), String> {
        let mut writer = self.inner.writer.lock().map_err(lock_error)?;
        let writer = writer.as_mut().ok_or("Codex app-server input is closed")?;
        serde_json::to_writer(&mut *writer, message)
            .map_err(|e| format!("Could not encode app-server request: {e}"))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|e| format!("Could not write to Codex app-server: {e}"))
    }
}

/// Locates the native Codex binary without invoking a command shell. This also
/// handles the common Windows npm installation where PATH exposes codex.cmd but
/// the actual executable lives below node_modules.
pub fn discover_codex_binary() -> Result<PathBuf, String> {
    resolve_codex_binary(Path::new("codex"))
}

fn resolve_codex_binary(requested: &Path) -> Result<PathBuf, String> {
    if requested.is_file() && is_native_executable(requested) {
        return Ok(requested.to_path_buf());
    }
    #[cfg(windows)]
    {
        let file_name = requested.file_name().and_then(|value| value.to_str());
        if requested.components().count() == 1 && matches!(file_name, Some("codex" | "codex.exe")) {
            for directory in std::env::var_os("PATH")
                .as_deref()
                .map(std::env::split_paths)
                .into_iter()
                .flatten()
            {
                let direct = directory.join("codex.exe");
                if direct.is_file() {
                    return Ok(direct);
                }
                if directory.join("codex.cmd").is_file() {
                    for package in ["codex-win32-x64", "codex-win32-arm64"] {
                        let native = directory
                            .join("node_modules")
                            .join("@openai")
                            .join("codex")
                            .join("node_modules")
                            .join("@openai")
                            .join(package)
                            .join("vendor")
                            .join(if package.ends_with("arm64") {
                                "aarch64-pc-windows-msvc"
                            } else {
                                "x86_64-pc-windows-msvc"
                            })
                            .join("bin")
                            .join("codex.exe");
                        if native.is_file() {
                            return Ok(native);
                        }
                    }
                }
            }
        }
        return Err(format!(
            "Could not locate the native codex.exe for {}",
            requested.display()
        ));
    }
    #[cfg(not(windows))]
    {
        // Unix Command resolves a bare executable through PATH without a shell.
        Ok(requested.to_path_buf())
    }
}

#[cfg(windows)]
fn is_native_executable(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

#[cfg(not(windows))]
fn is_native_executable(_: &Path) -> bool {
    true
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        if self.inner.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shutdown();
        }
    }
}

impl Clone for AppServerClient {
    fn clone(&self) -> Self {
        self.inner.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn checked_cwd(raw: &str) -> Result<String, String> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() || !path.is_dir() {
        return Err("Autopilot working directory must be an existing absolute directory".into());
    }
    path.canonicalize()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| format!("Could not resolve Autopilot working directory: {e}"))
}

fn sandbox_policy(policy: &ThreadPolicy) -> Value {
    if policy.writable {
        json!({
            "type": "workspaceWrite",
            "writableRoots": [policy.cwd],
            "networkAccess": false
        })
    } else {
        json!({"type": "readOnly", "networkAccess": false})
    }
}

fn human_response_payload(
    request: &HumanRequest,
    decision: HumanDecision,
) -> Result<Value, String> {
    match request.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decision = match decision {
                HumanDecision::Accept => "accept",
                HumanDecision::AcceptForSession => "acceptForSession",
                HumanDecision::Decline => "decline",
                HumanDecision::Cancel => "cancel",
                HumanDecision::Submit { .. } => {
                    return Err("This approval request requires an approve or deny decision".into())
                }
            };
            Ok(json!({"decision": decision}))
        }
        "item/permissions/requestApproval" => match decision {
            HumanDecision::Accept => Ok(json!({
                "permissions": request.params.get("permissions").cloned().unwrap_or_else(|| json!({})),
                "scope": "turn"
            })),
            HumanDecision::Decline | HumanDecision::Cancel => {
                Ok(json!({"permissions": {}, "scope": "turn"}))
            }
            HumanDecision::AcceptForSession => Err(
                "Session-wide permission grants are not supported; approve this turn explicitly"
                    .into(),
            ),
            HumanDecision::Submit { .. } => {
                Err("Permission requests require an approve or deny decision".into())
            }
        },
        "item/tool/requestUserInput" => match decision {
            HumanDecision::Submit { value } if value.is_object() => Ok(json!({"answers": value})),
            HumanDecision::Submit { .. } => Err("User input answers must be a JSON object".into()),
            _ => Err("This request requires answers from the user".into()),
        },
        "mcpServer/elicitation/request" => match decision {
            HumanDecision::Submit { value } => Ok(json!({"action": "accept", "content": value})),
            HumanDecision::Decline => Ok(json!({"action": "decline", "content": null})),
            HumanDecision::Cancel => Ok(json!({"action": "cancel", "content": null})),
            HumanDecision::Accept | HumanDecision::AcceptForSession => {
                Err("MCP elicitation acceptance requires submitted form content".into())
            }
        },
        _ => Err(format!(
            "Unsupported Codex human request type: {}. Cancel the turn to continue safely.",
            request.method
        )),
    }
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn receive_response(
    receiver: Receiver<Result<Value, String>>,
    timeout: Duration,
    method: &str,
) -> Result<Value, String> {
    receiver
        .recv_timeout(timeout)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => format!("Codex {method} request timed out"),
            mpsc::RecvTimeoutError::Disconnected => {
                format!("Codex app-server stopped during {method}")
            }
        })?
}

fn spawn_stdout_reader(inner: Arc<Inner>, stdout: impl Read + Send + 'static) {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => match serde_json::from_str::<Value>(&line) {
                    Ok(message) => process_message(&inner, message),
                    Err(error) => push_protocol_warning(
                        &inner,
                        format!("Invalid JSON from app-server: {error}"),
                    ),
                },
                Ok(_) => {}
                Err(error) => {
                    mark_dead(
                        &inner,
                        format!("Could not read Codex app-server output: {error}"),
                    );
                    return;
                }
            }
        }
        if !inner.shutting_down.load(Ordering::SeqCst) {
            mark_dead(&inner, "Codex app-server stopped unexpectedly".into());
        }
    });
}

fn process_message(inner: &Arc<Inner>, message: Value) {
    if message.get("method").is_none() {
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if let Ok(mut pending) = inner.pending.lock() {
                if let Some(sender) = pending.remove(&id) {
                    let response = if let Some(error) = message.get("error") {
                        Err(rpc_error(error))
                    } else {
                        Ok(message.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = sender.send(response);
                }
            }
        }
        return;
    }
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let request_id = message.get("id").cloned();
    let mut state = match inner.state.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    if let Some(request_id) = request_id {
        let request = HumanRequest {
            request_id,
            method: method.clone(),
            thread_id: string_field(&params, "threadId"),
            turn_id: string_field(&params, "turnId"),
            params: params.clone(),
        };
        attach_human_request(&mut state, request);
    } else {
        apply_notification(&mut state, &method, &params);
    }
    push_app_server_event(&mut state.events, AppServerEvent { method, params });
    drop(state);
    inner.changed.notify_all();
}

fn attach_human_request(state: &mut SharedState, request: HumanRequest) {
    if let Some(turn_id) = request.turn_id.clone() {
        let turn = state.turns.entry(turn_id).or_default();
        if turn.thread_id.is_empty() {
            turn.thread_id = request.thread_id.clone().unwrap_or_default();
        }
        turn.human_requests.push(request);
        return;
    }
    if let Some((_, turn)) = state
        .turns
        .iter_mut()
        .find(|(_, turn)| request.thread_id.as_deref() == Some(turn.thread_id.as_str()))
    {
        turn.human_requests.push(request);
        return;
    }
    // Unknown/new request types may omit scoping fields. Fail closed by
    // surfacing the gate on every active turn instead of silently hanging.
    for turn in state
        .turns
        .values_mut()
        .filter(|turn| turn.status.is_none())
    {
        turn.human_requests.push(request.clone());
    }
}

fn apply_notification(state: &mut SharedState, method: &str, params: &Value) {
    if method == "serverRequest/resolved" {
        if let Some(request_id) = params.get("requestId") {
            remove_pending_request_id(state, request_id);
        }
        return;
    }
    let turn_id = string_field(params, "turnId").or_else(|| {
        params
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let Some(turn_id) = turn_id else { return };
    let turn = state.turns.entry(turn_id).or_default();
    if turn.thread_id.is_empty() {
        turn.thread_id = string_field(params, "threadId").unwrap_or_default();
    }
    match method {
        "item/completed" => {
            let item = params.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) == Some("agentMessage")
                && item.get("phase").and_then(Value::as_str) == Some("final_answer")
            {
                turn.structured_output = item
                    .get("text")
                    .and_then(Value::as_str)
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .filter(valid_structured_output);
            }
        }
        "thread/tokenUsage/updated" => {
            let usage = params.pointer("/tokenUsage/last").unwrap_or(&Value::Null);
            turn.input_tokens = usage
                .get("inputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(turn.input_tokens);
            turn.output_tokens = usage
                .get("outputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(turn.output_tokens);
        }
        "turn/completed" => {
            turn.status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some("failed".into()));
            turn.error = params
                .pointer("/turn/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if turn.status.as_deref() == Some("completed") && turn.structured_output.is_none() {
                turn.status = Some("failed".into());
                turn.error = Some("Codex completed without a valid structured final answer".into());
            }
            // Completion invalidates every outstanding request for the turn.
            // A late UI response must never be written to a finished turn.
            turn.human_requests.clear();
        }
        _ => {}
    }
}

fn remove_pending_request_id(state: &mut SharedState, request_id: &Value) {
    for turn in state.turns.values_mut() {
        turn.human_requests
            .retain(|pending| &pending.request_id != request_id);
    }
}

fn take_pending_human_request(
    state: &mut SharedState,
    request: &HumanRequest,
) -> Result<(), String> {
    if let Some(turn_id) = request.turn_id.as_deref() {
        if let Some(turn) = state.turns.get(turn_id) {
            if turn.status.is_some() {
                return Err(format!(
                    "Codex turn {turn_id} is already terminal; this request is stale"
                ));
            }
        }
    }
    let exact_matches = state
        .turns
        .values()
        .flat_map(|turn| turn.human_requests.iter())
        .filter(|pending| *pending == request)
        .count();
    if exact_matches == 0 {
        return Err("This Codex request is no longer pending or its identity changed".into());
    }
    // An unscoped unknown request may be mirrored onto several active turns.
    // One explicit answer consumes every identical mirror exactly once.
    for turn in state.turns.values_mut() {
        turn.human_requests.retain(|pending| pending != request);
    }
    Ok(())
}

fn valid_structured_output(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    matches!(
        object.get("outcome").and_then(Value::as_str),
        Some("pass" | "needs_input" | "fail")
    ) && object.get("summary").and_then(Value::as_str).is_some()
}

fn turn_result(turn_id: &str, turn: &TurnState, status: &str) -> ManagedTurnResult {
    ManagedTurnResult {
        thread_id: turn.thread_id.clone(),
        turn_id: turn_id.to_owned(),
        status: status.to_owned(),
        structured_output: turn.structured_output.clone(),
        input_tokens: turn.input_tokens,
        output_tokens: turn.output_tokens,
        error: turn.error.clone(),
        human_requests: turn.human_requests.clone(),
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn rpc_error(error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown app-server error");
    match code {
        Some(code) => format!("Codex app-server error {code}: {message}"),
        None => format!("Codex app-server error: {message}"),
    }
}

fn push_protocol_warning(inner: &Arc<Inner>, warning: String) {
    if let Ok(mut state) = inner.state.lock() {
        push_app_server_event(
            &mut state.events,
            AppServerEvent {
                method: "transport/warning".into(),
                params: json!({"message": warning}),
            },
        );
    }
    inner.changed.notify_all();
}

fn push_app_server_event(events: &mut VecDeque<AppServerEvent>, mut incoming: AppServerEvent) {
    let incoming_is_delta = is_stream_delta(&incoming.method);
    if incoming_is_delta {
        let incoming_turn = event_scope(&incoming.params, "turnId", "/turn/id");
        let incoming_item = event_scope(&incoming.params, "itemId", "/item/id");
        if let Some(existing) = events.iter_mut().rev().find(|event| {
            event.method == incoming.method
                && event_scope(&event.params, "turnId", "/turn/id") == incoming_turn
                && event_scope(&event.params, "itemId", "/item/id") == incoming_item
        }) {
            if let (Some(previous), Some(next)) = (
                existing.params.get("delta").and_then(Value::as_str),
                incoming.params.get("delta").and_then(Value::as_str),
            ) {
                let merged = previous
                    .chars()
                    .chain(next.chars())
                    .take(MAX_COALESCED_DELTA_CHARS)
                    .collect::<String>();
                incoming.params["delta"] = Value::String(merged);
            }
            // Coalesce a bounded preview before drain so chunk boundaries remain
            // readable, while one hot stream cannot starve terminal events.
            *existing = incoming;
            return;
        }
    }

    if events.len() >= MAX_EVENT_BACKLOG {
        if let Some(delta_index) = events
            .iter()
            .position(|event| is_stream_delta(&event.method))
        {
            events.remove(delta_index);
        } else if incoming_is_delta {
            return;
        } else {
            events.pop_front();
        }
    }
    events.push_back(incoming);
}

fn is_stream_delta(method: &str) -> bool {
    method.ends_with("/delta") || method.ends_with("/outputDelta")
}

fn event_scope(params: &Value, field: &str, pointer: &str) -> Option<String> {
    params
        .get(field)
        .and_then(Value::as_str)
        .or_else(|| params.pointer(pointer).and_then(Value::as_str))
        .map(str::to_owned)
}

fn mark_dead(inner: &Arc<Inner>, error: String) {
    if let Ok(mut state) = inner.state.lock() {
        state.dead = Some(error.clone());
    }
    if let Ok(mut pending) = inner.pending.lock() {
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(error.clone()));
        }
    }
    inner.changed.notify_all();
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "Codex app-server transport lock is poisoned".into()
}

#[cfg(windows)]
fn hide_windows_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_windows_console(_: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn test_inner() -> Arc<Inner> {
        Arc::new(Inner {
            writer: Mutex::new(None),
            child: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            thread_policies: Mutex::new(HashMap::new()),
            state: Mutex::new(SharedState::default()),
            changed: Condvar::new(),
            next_id: AtomicU64::new(1),
            handles: AtomicUsize::new(1),
            shutting_down: AtomicBool::new(false),
            timeout: Duration::from_secs(1),
            initialize_result: Mutex::new(Value::Null),
        })
    }

    fn client_with_capture() -> (AppServerClient, Arc<Inner>, Arc<Mutex<Vec<u8>>>) {
        let inner = test_inner();
        let capture = Arc::new(Mutex::new(Vec::new()));
        *inner.writer.lock().unwrap() = Some(Box::new(CaptureWriter(Arc::clone(&capture))));
        let client = AppServerClient {
            inner: Arc::clone(&inner),
        };
        (client, inner, capture)
    }

    #[test]
    fn routes_rpc_responses_to_the_matching_request() {
        let inner = test_inner();
        let (send, receive) = mpsc::sync_channel(1);
        inner.pending.lock().unwrap().insert(42, send);
        process_message(&inner, json!({"id": 42, "result": {"ok": true}}));
        assert_eq!(receive.recv().unwrap().unwrap(), json!({"ok": true}));
        assert!(inner.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn approval_request_is_a_human_gate_and_is_never_answered() {
        let inner = test_inner();
        process_message(
            &inner,
            json!({"method":"turn/started","params":{"threadId":"th","turn":{"id":"tu"}}}),
        );
        process_message(
            &inner,
            json!({
                "id": 91,
                "method": "item/commandExecution/requestApproval",
                "params": {"threadId":"th","turnId":"tu","command":"cargo test"}
            }),
        );
        let state = inner.state.lock().unwrap();
        let turn = state.turns.get("tu").unwrap();
        assert_eq!(turn.human_requests.len(), 1);
        assert_eq!(turn.human_requests[0].request_id, json!(91));
        assert!(inner.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn completion_requires_valid_structured_final_answer() {
        let inner = test_inner();
        process_message(
            &inner,
            json!({"method":"turn/started","params":{"threadId":"th","turn":{"id":"tu"}}}),
        );
        process_message(
            &inner,
            json!({"method":"item/completed","params":{"threadId":"th","turnId":"tu","item":{
                "type":"agentMessage","phase":"final_answer","text":"{\"outcome\":\"pass\",\"summary\":\"done\"}"
            }}}),
        );
        process_message(
            &inner,
            json!({"method":"turn/completed","params":{"threadId":"th","turn":{"id":"tu","status":"completed"}}}),
        );
        let state = inner.state.lock().unwrap();
        let turn = state.turns.get("tu").unwrap();
        assert_eq!(turn.status.as_deref(), Some("completed"));
        assert_eq!(turn.structured_output.as_ref().unwrap()["outcome"], "pass");
    }

    #[test]
    fn invalid_final_answer_turn_is_failed_closed() {
        let inner = test_inner();
        process_message(
            &inner,
            json!({"method":"turn/completed","params":{"threadId":"th","turn":{"id":"tu","status":"completed"}}}),
        );
        let state = inner.state.lock().unwrap();
        let turn = state.turns.get("tu").unwrap();
        assert_eq!(turn.status.as_deref(), Some("failed"));
        assert!(turn.error.as_deref().unwrap().contains("structured"));
    }

    #[test]
    fn token_usage_uses_last_turn_breakdown() {
        let inner = test_inner();
        process_message(
            &inner,
            json!({"method":"thread/tokenUsage/updated","params":{
                "threadId":"th","turnId":"tu","tokenUsage":{
                    "last":{"inputTokens":12,"outputTokens":7},
                    "total":{"inputTokens":99,"outputTokens":88}
                }
            }}),
        );
        let state = inner.state.lock().unwrap();
        let turn = state.turns.get("tu").unwrap();
        assert_eq!((turn.input_tokens, turn.output_tokens), (12, 7));
    }

    #[test]
    fn structured_contract_rejects_unknown_outcome_or_missing_summary() {
        assert!(valid_structured_output(
            &json!({"outcome":"needs_input","summary":"choose"})
        ));
        assert!(!valid_structured_output(
            &json!({"outcome":"maybe","summary":"x"})
        ));
        assert!(!valid_structured_output(&json!({"outcome":"pass"})));
    }

    #[test]
    fn managed_sandbox_never_inherits_network_access() {
        let writable = sandbox_policy(&ThreadPolicy {
            cwd: "C:/project".into(),
            writable: true,
        });
        assert_eq!(writable["type"], "workspaceWrite");
        assert_eq!(writable["networkAccess"], false);
        assert_eq!(writable["writableRoots"], json!(["C:/project"]));
        let read_only = sandbox_policy(&ThreadPolicy {
            cwd: "C:/project".into(),
            writable: false,
        });
        assert_eq!(read_only, json!({"type":"readOnly","networkAccess":false}));
    }

    #[test]
    fn human_decisions_are_scoped_and_explicit() {
        let command = HumanRequest {
            request_id: json!(4),
            method: "item/commandExecution/requestApproval".into(),
            thread_id: Some("th".into()),
            turn_id: Some("tu".into()),
            params: json!({"command":"cargo test"}),
        };
        assert_eq!(
            human_response_payload(&command, HumanDecision::Accept).unwrap(),
            json!({"decision":"accept"})
        );
        let permission = HumanRequest {
            request_id: json!(5),
            method: "item/permissions/requestApproval".into(),
            thread_id: Some("th".into()),
            turn_id: Some("tu".into()),
            params: json!({"permissions":{"network":{"enabled":true}}}),
        };
        assert_eq!(
            human_response_payload(&permission, HumanDecision::Accept).unwrap(),
            json!({"permissions":{"network":{"enabled":true}},"scope":"turn"})
        );
        assert!(human_response_payload(&permission, HumanDecision::AcceptForSession).is_err());
    }

    #[test]
    fn unknown_unscoped_request_gates_all_active_turns() {
        let inner = test_inner();
        process_message(
            &inner,
            json!({"method":"turn/started","params":{"threadId":"th","turn":{"id":"tu"}}}),
        );
        process_message(
            &inner,
            json!({"id":"opaque","method":"future/request","params":{"message":"choose"}}),
        );
        let state = inner.state.lock().unwrap();
        assert_eq!(state.turns["tu"].human_requests[0].method, "future/request");
    }

    #[test]
    fn last_public_handle_shuts_transport_down_despite_reader_arc() {
        let inner = test_inner();
        let client = AppServerClient {
            inner: Arc::clone(&inner),
        };
        let second = client.clone();
        drop(second);
        assert!(!inner.shutting_down.load(Ordering::SeqCst));
        drop(client);
        assert!(inner.shutting_down.load(Ordering::SeqCst));
        assert_eq!(inner.handles.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shutdown_wakes_pending_requests_immediately() {
        let inner = test_inner();
        let client = AppServerClient {
            inner: Arc::clone(&inner),
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        inner.pending.lock().unwrap().insert(7, sender);
        client.shutdown();
        assert!(receiver
            .recv_timeout(Duration::from_millis(50))
            .unwrap()
            .is_err());
        assert!(inner.pending.lock().unwrap().is_empty());
    }

    fn pending_command(inner: &Arc<Inner>) -> HumanRequest {
        process_message(
            inner,
            json!({"method":"turn/started","params":{"threadId":"th","turn":{"id":"tu"}}}),
        );
        process_message(
            inner,
            json!({
                "id": 91,
                "method": "item/commandExecution/requestApproval",
                "params": {"threadId":"th","turnId":"tu","command":"cargo test"}
            }),
        );
        inner.state.lock().unwrap().turns["tu"].human_requests[0].clone()
    }

    #[test]
    fn mismatched_request_identity_is_rejected_before_write() {
        let (client, inner, capture) = client_with_capture();
        let request = pending_command(&inner);
        for forged in [
            HumanRequest {
                request_id: json!(92),
                ..request.clone()
            },
            HumanRequest {
                method: "item/fileChange/requestApproval".into(),
                ..request.clone()
            },
            HumanRequest {
                thread_id: Some("other-thread".into()),
                ..request.clone()
            },
            HumanRequest {
                turn_id: Some("other-turn".into()),
                ..request.clone()
            },
            HumanRequest {
                params: json!({"threadId":"th","turnId":"tu","command":"other"}),
                ..request.clone()
            },
        ] {
            assert!(client
                .respond_to_human_request(&forged, HumanDecision::Accept)
                .is_err());
            assert!(capture.lock().unwrap().is_empty());
        }
        assert_eq!(
            inner.state.lock().unwrap().turns["tu"].human_requests.len(),
            1
        );
    }

    #[test]
    fn resolved_and_terminal_requests_are_rejected_before_write() {
        let (client, inner, capture) = client_with_capture();
        let request = pending_command(&inner);
        process_message(
            &inner,
            json!({"method":"serverRequest/resolved","params":{"threadId":"th","requestId":91}}),
        );
        assert!(client
            .respond_to_human_request(&request, HumanDecision::Accept)
            .is_err());
        assert!(capture.lock().unwrap().is_empty());

        let request = pending_command(&inner);
        process_message(
            &inner,
            json!({"method":"turn/completed","params":{"threadId":"th","turn":{"id":"tu","status":"failed","error":{"message":"cancelled"}}}}),
        );
        assert!(client
            .respond_to_human_request(&request, HumanDecision::Accept)
            .is_err());
        assert!(capture.lock().unwrap().is_empty());
        assert!(inner.state.lock().unwrap().turns["tu"]
            .human_requests
            .is_empty());
    }

    #[test]
    fn duplicate_human_response_writes_exactly_once() {
        let (client, inner, capture) = client_with_capture();
        let request = pending_command(&inner);
        client
            .respond_to_human_request(&request, HumanDecision::Decline)
            .unwrap();
        let written_once = capture.lock().unwrap().clone();
        assert!(!written_once.is_empty());
        assert!(client
            .respond_to_human_request(&request, HumanDecision::Decline)
            .is_err());
        assert_eq!(*capture.lock().unwrap(), written_once);
    }

    #[test]
    fn delta_backlog_never_evicts_terminal_or_human_lifecycle() {
        let mut events = VecDeque::new();
        for index in 0..MAX_EVENT_BACKLOG {
            push_app_server_event(
                &mut events,
                AppServerEvent {
                    method: "item/agentMessage/delta".into(),
                    params: json!({"turnId":"turn","itemId":format!("item-{index}"),"delta":"chunk"}),
                },
            );
        }
        push_app_server_event(
            &mut events,
            AppServerEvent {
                method: "item/commandExecution/requestApproval".into(),
                params: json!({"turnId":"turn","command":"npm test"}),
            },
        );
        push_app_server_event(
            &mut events,
            AppServerEvent {
                method: "turn/completed".into(),
                params: json!({"turn":{"id":"turn","status":"completed"}}),
            },
        );

        assert_eq!(events.len(), MAX_EVENT_BACKLOG);
        assert!(events
            .iter()
            .any(|event| event.method == "item/commandExecution/requestApproval"));
        assert!(events.iter().any(|event| event.method == "turn/completed"));
    }

    #[test]
    fn adjacent_delta_chunks_keep_boundaries_and_unicode() {
        let mut events = VecDeque::new();
        for delta in ["Xin ", "chào ", "🌍"] {
            push_app_server_event(
                &mut events,
                AppServerEvent {
                    method: "item/agentMessage/delta".into(),
                    params: json!({"turnId":"turn","itemId":"message","delta":delta}),
                },
            );
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].params["delta"], "Xin chào 🌍");
    }
}
