# Agent Control Center 0.3.0

Ứng dụng desktop cho Windows để theo dõi đội agent Codex và chạy kế hoạch Software Team qua Autopilot. Giao diện hỗ trợ light/dark, sidebar thu gọn và thay đổi kích thước cửa sổ. Tên **Autopilot V2** trong giao diện là tên tính năng; phiên bản ứng dụng hiện tại là **0.3.0**.

## Chức năng dùng ngay

| Trang / chức năng | Cách sử dụng |
| --- | --- |
| **Overview** | Xem role, model, task, trạng thái quan sát, runtime, lệnh/file gần nhất và token khi dữ liệu có sẵn. |
| **Workflow** | Xem luồng Planner → Coder/Mobile Dev → Tester → Reviewer; chọn role để xem bằng chứng và sự kiện liên quan. |
| **Activity log** | Tìm kiếm nội dung, lọc role/loại sự kiện, mở nhóm để xem từng bản ghi và tải lịch sử cũ. |
| **Autopilot** | Tạo task, khai báo dependency, duyệt revision rồi chạy hàng đợi theo thứ tự phụ thuộc. |
| **Human direction** | Trả lời khi agent cần hướng dẫn, xử lý yêu cầu đang chờ và duyệt kết quả tại bước human review. |
| **Run controls** | Tạm dừng giao task mới, tiếp tục hoặc yêu cầu hủy công việc do controller quản lý. |
| **Model settings** | Chọn model và reasoning cho Planner, Coder, Mobile Dev, Tester, Reviewer; lưu riêng từng role. |
| **Research Team** | Theo dõi các role nghiên cứu và chỉ số tài liệu trong workspace phù hợp; hiện chỉ giám sát. |
| **Giao diện** | Dùng nút menu để đóng/mở sidebar; nút mặt trời/mặt trăng để đổi theme. Lựa chọn được lưu trên máy. |

## Bắt đầu sử dụng

### Mở ứng dụng Windows

1. Đóng bản Agent Control Center cũ trước khi mở bản mới.
2. Chạy `Agent-Control-Center-0.3.0-x64.exe`, hoặc cài bằng `Agent-Control-Center-0.3.0-x64-setup.exe` nếu đã có bộ phát hành. Các file này là sản phẩm build riêng, không nằm trong mã nguồn Git.
3. Mở **Overview** và kiểm tra kết nối. Monitor đọc session Codex đã có trên máy; role chưa có dữ liệu sẽ hiện thông tin chưa khả dụng.
4. Chọn **Software Team** hoặc **Research Team** ở sidebar. Mở **Workflow** để xem từng role hoặc **Activity log** để tìm sự kiện.

Máy cần Edge WebView2 để chạy giao diện. Để dùng Autopilot, cần Codex CLI đã đăng nhập và có trong PATH của tiến trình desktop. Không cần Node.js/Rust chỉ để chạy EXE đã build; các công cụ đó cần khi phát triển hoặc tự build từ source.

### Chạy một kế hoạch Autopilot

1. Mở **Autopilot**, nhập tên kế hoạch, đường dẫn workspace và yêu cầu có tiêu chí hoàn thành rõ ràng.
2. Dùng **Apply Software Team template** hoặc **Add task** để tạo các bước. Dependency là ID task, nhiều ID được phân cách bằng dấu phẩy.
3. Kiểm tra task, dependency, model, số lần thử và giới hạn token/thời gian. Giữ các bước Tester, Reviewer và Human final review phù hợp với quy trình.
4. Chọn **Save draft → Approve revision → Start Autopilot**. Sửa kế hoạch đã lưu sẽ làm mất hiệu lực phê duyệt cũ.
5. Theo dõi **Managed task evidence** và activity của controller. Khi có yêu cầu, trả lời trong **Human direction**; bước duyệt cuối cần quyết định của bạn.

Ví dụ yêu cầu nhỏ để bắt đầu:

> Thêm kiểm tra trường email trong form liên hệ. Email sai định dạng phải hiện thông báo bên dưới ô nhập; email hợp lệ được chấp nhận. Giữ bố cục hiện có, bổ sung kiểm thử phù hợp và dừng tại Human final review.

Autopilot hiện chạy **một task mỗi lần**, dùng chung workspace và cần giữ app mở. **Pause after task** ngăn giao task tiếp theo; task đang chạy có thể tiếp tục. **Request cancel** yêu cầu dừng công việc đang quản lý, không hoàn tác file đã sửa. Khi phục hồi sau gián đoạn, controller cần quyết định rõ ràng trước khi tiếp tục.

### Đổi model cho agent

1. Mở **Model settings**, chọn model và reasoning cho role cần đổi.
2. Chọn **Save role**; ứng dụng báo đường dẫn bản sao lưu khi ghi thành công.
3. Bắt đầu phiên mới để dùng mặc định đã lưu. Phiên đang chạy giữ model hiện tại; cấu hình project có thể ghi đè mặc định global.
4. Với Autopilot, kiểm tra lại model/effort đã lưu trong kế hoạch trước khi duyệt và chạy.

Các lựa chọn model lấy từ catalog cache trên máy, không bảo đảm tài khoản có quyền sử dụng tất cả lựa chọn. Thay đổi Planner là thay đổi mặc định global cho các phiên Codex tương lai, kể cả ngoài ứng dụng này.

### Hiểu trạng thái agent

- **Recent**: có bằng chứng trạng thái trong hai phút gần nhất.
- **Last observed**: trạng thái được ghi nhận trước đó; chưa có bằng chứng mới đủ gần.
- **Time unknown**: chưa xác định được thời điểm trạng thái hợp lệ.
- **Tokens unavailable / Model unavailable**: nguồn log chưa cung cấp đủ thông tin để gán chính xác cho role.

Một lệnh chạy lâu có thể im lặng. Trạng thái cũ không chứng minh agent đã dừng; dispatch cũng không chứng minh subagent đang chạy. Bộ đếm **Recent working observations** chỉ tính Working được quan sát gần đây. Tổng **Observed tokens** bao gồm tất cả session được monitor, không chỉ team đang chọn; **Managed tokens** thuộc kế hoạch Autopilot.

### Phân biệt hai kết nối

| Chỉ báo | Ý nghĩa |
| --- | --- |
| **Codex connected / Local Codex events** | Monitor truy cập được thư mục session local; không xác nhận kết nối điều khiển phiên Codex bên ngoài. |
| **Codex App Server ready** trong Autopilot | Controller đã kết nối App Server riêng để quản lý task của kế hoạch. |

Nếu không thấy subagent hoạt động, kiểm tra **Last observed**, thread và sự kiện liên quan trong Workflow/Activity. Monitor cần log trực tiếp của subagent hoặc snapshot trạng thái có cấu trúc từ parent. App không điều khiển các phiên bên ngoài chỉ bằng việc đọc log.

## Technical reference

Windows-first Tauri desktop application with **Software Team** and **Research Team** profiles. The Overview, Workflow, and Activity pages monitor local sessions. The separate **Autopilot** page controls explicitly approved Software Team plans using its own Codex App Server connection. Research remains monitor-only. Model Settings can update selected Software Team global defaults only after the user selects **Save role**.

## Software Autopilot

Open **Autopilot**, enter the target workspace and requirements, and prepare a dependency map using the Software Team template or individual tasks. Inspect every task and dependency, save the draft, approve its revision, then select **Start Autopilot**. Editing the saved plan invalidates its approval.

The controller uses the installed `codex app-server` over local stdio, with the existing Codex sign-in. Managed tasks have their own thread and turn identifiers. Completion is based on App Server lifecycle events and validated structured results; monitor log inference never advances this queue. The connection in Autopilot is separate from the monitor's session-directory availability indicator.

V2 executes one task at a time in a shared workspace. Dependency branches are supported, but parallel writes and isolated worktrees are not implemented. Tester and Reviewer checks precede final human review. Human direction is required when a task cannot proceed or a run needs recovery. Token and time ceilings stop further work when observed; they are not prepaid billing limits.

The initial map is editable and template-based; V2 does not yet generate a dependency graph from natural language. Planner is a managed task in the approved graph. Managed execution uses the model and effort saved with the plan. Review those values before starting after changing Model Settings.

Managed state and recent controller events are stored in `%LOCALAPPDATA%\AgentControlCenter\autopilot.json`, separately from monitor SQLite history. Do not edit that file while the application is open. The controller keeps one current plan; saving a revised plan resets its execution state and requires approval again.

Pause stops new dispatch; review the active task status before changing files. A cancellation request enters **Cancelling** while the controller stops owned work. Cancelling does not undo file changes already made by agents. Restart recovery requires an explicit decision and does not automatically replay interrupted work; old protocol approvals are not reused on a new connection.

The app must be open for managed execution. It does not install a background service or silently start work on launch. Codex CLI must be on the desktop process PATH and already authenticated; this app does not collect API keys or edit authentication files.

## License

This project is licensed under the [MIT License](LICENSE).

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
npm run tauri dev
```

After a release build, launch the Research profile with the supplied Windows helper:

```powershell
.\start-research-monitor.ps1 -ResearchWorkspace 'C:\path\to\Research-Team'
```

Build an installer:

```powershell
npm run tauri build
```

Frontend checks:

```powershell
npm run typecheck
npm run test:frontend
npm run build
```

Opening the Vite frontend alone will show a connecting state because the live monitor runs in the Tauri Rust process.

## Model settings

Open **Model settings** from the sidebar to set the global Codex model and reasoning effort for Planner, Coder, Mobile Dev, Tester, and Reviewer. The choices come from the locally cached model catalog. A cached catalog does not verify that an account can use a model.

Changing a selection only creates an in-app draft. Save each role separately to write its global default; each successful change reports its backup path. Existing Codex sessions keep their current model. Start a new session to use a saved default. Project configuration or an explicit task setting can override a global default. Planner is a global default for future Codex sessions, including sessions outside this dashboard's Software Team profile.

The Research Team remains monitor-only. Reload discards unsaved drafts and reads the current settings again.

## Workflow and activity history

**Workflow** is a live role view. For Software Team it shows Planner → Coder/Mobile Dev → Tester → Reviewer; Research Team shows its monitored research sequence and keeps approval gates manual. Select a role to inspect its latest observed task, model, status, runtime, file or command, workspace, thread, token availability, and related stored events. Role cards can reflect different sessions or workspaces and workflow arrows do not prove that one role handed work to another.

**Activity log** reads retained SQLite event summaries. It supports text, role, and event-type filters and loads older pages. Adjacent events with the same role, thread, and kind are grouped when consecutive timestamps are at most 30 seconds apart. Expand a group to inspect every retained summary, thread, and database record ID. Distinct errors, approvals, and lifecycle milestones remain separate. The list is newest recorded first, by database insertion order rather than event timestamp. New monitor updates refresh the newest page after a short delay. When browsing older pages, automatic refresh pauses until **Refresh** is selected.

Agent cards and Workflow show the latest available command, file, event, or dispatch evidence. Status remains the last lifecycle state observed in Codex logs. Status evidence has its own source timestamp, separate from general activity. **Recent working observations** counts only Working observations within the last two minutes. Older evidence is labelled **last observed** and missing or invalid status timestamps are labelled **time unknown**. Only recently observed Working roles have a ticking runtime. Silence can mean a long-running command; the freshness label does not prove that an agent stopped. Missing sessions are labelled **No session observed**.

On this machine, `npm run typecheck`, `npm run build`, `cargo check`, and `cargo test` have passed. Install the prerequisites above on any new machine, then run `npm run tauri dev` and `npm run tauri build` to compile and verify the desktop executable.

## Key files

- `src/main.tsx`: dashboard and live update subscription
- `src/style.css`: dark/light responsive visual design
- `src-tauri/src/lib.rs`: event reader, AgentState normalization, SQLite activity history, Tauri command/event
- `src/AutopilotPage.tsx`: plan editor, execution controls, human queue and managed activity
- `src-tauri/src/autopilot.rs`: persistent DAG state machine, quality gates and retry limits
- `src-tauri/src/app_server.rs`: official Codex App Server stdio protocol client
- `src-tauri/tauri.conf.json`: Windows app configuration

## Current limits

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
