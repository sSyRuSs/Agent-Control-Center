# Agent Control Center V1.5

Windows-first Tauri desktop monitor with parallel **Software Team** and **Research Team** profiles. Monitoring does not start agents, pass human approval gates, or change agent turns. Model Settings can update selected Software Team global defaults only after the user selects **Save role**.

## License

This project is licensed under the [MIT License](../LICENSE).

## Data source

The desktop application tails the real Codex JSONL session event files under `%USERPROFILE%\.codex\sessions` (or `$CODEX_HOME\sessions` when set) every two seconds. It parses session metadata, turn and item events, user messages, tool calls, model, and token usage. Profile cards show the latest observed session for each configured role. Root sessions are assigned to Planner; internal Codex subagents such as `guardian` are ignored. Activity metadata is saved in `%LOCALAPPDATA%\AgentControlCenter\history.sqlite3` with a stable event key so rescans and later launches do not add duplicate rows.

The Research Team profile recognizes `literature_researcher`, `data_evidence`, `methodology_scientist`, `experiment_engineer`, `paper_writer`, and `academic_reviewer`, plus a root session whose workspace path contains `Research-Team`. It reads the research workspace matrices, experiment configs, paper sections, and review status to calculate the research indicators. Set `AGENT_CONTROL_RESEARCH_WORKSPACE` when starting the app to show artifact metrics before the first Research Team session is detected.

This machine's Codex CLI supports the official App Server, but a separately spawned App Server cannot subscribe to the already running desktop application's transport. V1 therefore uses the local session event stream as its live source. The connection indicator means the session directory is available; it does not claim an App Server transport connection. The workflow arrows describe the intended role order and do not prove a handoff. Statuses are inferred from observed events. If an event does not provide a task, file, or command, the card says so instead of inventing one.

When Codex records collaboration only in the parent rollout, V1 reads the structured `spawn_agent` arguments to show an exact delegated role and its dispatched task. A subsequent structured `list_agents` result updates that card from the reported agent status. The parent rollout does not provide per-child token counters, so those cards show `Tokens unavailable` and are excluded from the observed-token total until a child session supplies its own usage records.

Delegated status changes require a structured `list_agents` snapshot or a direct child session. A spawn or follow-up proves dispatch only; it cannot prove that the child is currently running. Each card shows the freshest observed parent cohort, so an absent later lifecycle event remains a last observed state rather than a live execution guarantee.

The parent rollout can also multiplex child turn context and usage under the root session. V1 therefore leaves the Planner model and tokens unavailable when that association cannot be proven, rather than attributing a child model or token count to Planner. The `Observed tokens` metric counts the deduplicated token stream across all monitored sessions and remains global when changing team profiles.

## Run

Requirements: Node.js 20+, Rust MSVC toolchain, Microsoft C++ Build Tools with Desktop development with C++, and Edge WebView2. See [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/).

```powershell
npm install
$env:AGENT_CONTROL_RESEARCH_WORKSPACE = 'C:\Users\ADMIN\Documents\Codex\2026-09-19\referenced-chatgpt-conversation-this-is-an\outputs\Research-Team'
npm run tauri dev
```

After a release build, launch the Research profile with the supplied Windows helper:

```powershell
.\start-research-monitor.ps1
```

Build an installer:

```powershell
npm run tauri build
```

Frontend checks:

```powershell
npm run typecheck
npm run build
```

Opening the Vite frontend alone will show a connecting state because the live monitor runs in the Tauri Rust process.

## Model settings

Open **Model settings** from the sidebar to set the global Codex model and reasoning effort for Planner, Coder, Mobile Dev, Tester, and Reviewer. The choices come from the locally cached model catalog. A cached catalog does not verify that an account can use a model.

Changing a selection only creates an in-app draft. Save each role separately to write its global default; each successful change reports its backup path. Existing Codex sessions keep their current model. Start a new session to use a saved default. Project configuration or an explicit task setting can override a global default. Planner is a global default for future Codex sessions, including sessions outside this dashboard's Software Team profile.

The Research Team remains monitor-only. Reload discards unsaved drafts and reads the current settings again.

## Workflow and activity history

**Workflow** is a live role view. For Software Team it shows Planner → Coder/Mobile Dev → Tester → Reviewer; Research Team shows its monitored research sequence and keeps approval gates manual. Select a role to inspect its latest observed task, model, status, runtime, file or command, workspace, thread, token availability, and related stored events. Role cards can reflect different sessions or workspaces and workflow arrows do not prove that one role handed work to another.

**Activity log** reads retained SQLite event summaries. It supports text, role, and event-type filters; expands a stored event summary with its recorded thread and database record ID; and loads older pages. The list is newest recorded first, by database insertion order rather than event timestamp. New monitor updates refresh the newest page after a short delay. When browsing older pages, automatic refresh pauses until **Refresh** is selected.

Agent cards and Workflow now show **Last observed** and the latest available command, file, event, or dispatch evidence. Status remains the last lifecycle state observed in Codex logs. A running duration ticks only for a `Working` lifecycle; completed, failed, and blocked roles show their observed duration, while idle and waiting roles show the age of their most recent observation. Missing observations are labelled **No session observed** rather than inferred as active.

On this machine, `npm run typecheck`, `npm run build`, `cargo check`, and `cargo test` have passed. Install the prerequisites above on any new machine, then run `npm run tauri dev` and `npm run tauri build` to compile and verify the desktop executable.

## Key files

- `src/main.tsx`: dashboard and live update subscription
- `src/style.css`: dark/light responsive visual design
- `src-tauri/src/lib.rs`: event reader, AgentState normalization, SQLite activity history, Tauri command/event
- `src-tauri/tauri.conf.json`: Windows app configuration

## V1.5 limits

- Role matching uses Codex subagent metadata and root-session Planner attribution. The Software/Research selector separates the profiles, but several projects using the same role can still resolve to the most recently active session.
- History is stored in SQLite as deduplicated activity metadata and the latest 120 entries are restored when the app opens. The dashboard rebuilds its live card state from recent Codex logs when it starts.
- Activity events are sanitized stored summaries, not raw Codex payloads. The role history in Workflow matches a selected thread where an event recorded one; otherwise it can span sessions.
- Model Settings manages global defaults only; it does not change a running agent, select project overrides, or make account access claims.
- Research CSV metric parsing expects the provided matrix column order and simple comma-delimited values; use a stable export when fields contain embedded commas.

## Mobile Dev integration

Software Team has five roles: Planner, Coder, Mobile Dev, Tester, Reviewer.
Planner routes web/backend/general tasks to Coder; Flutter/Dart/Android/iOS tasks
to `mobile-dev`; cross-platform work can use both with separate ownership.
Both implementation branches converge at Tester, then independent Reviewer.
Mobile Dev's original machine default was `gpt-6-sol` / `medium`, workspace-write.
Use Model Settings to inspect or update the current global default. Difficult architecture,
security, concurrency and native-platform issues escalate through Planner only.
The monitor recognizes `mobile-dev` and `mobile_dev` session metadata, using the same
live status, task, runtime and token fields as other roles. Cards show an observed
session model only when the local event stream provides one.
Monitoring is read-only and is not an enforcement engine for review gates.
Restart the rebuilt desktop executable to load the updated dashboard.
