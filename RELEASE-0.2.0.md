# Agent Control Center 0.2.0 — Autopilot V2

## Chức năng

- Software Team: dependency map chỉnh sửa được; Save draft → Approve revision → Start Autopilot.
- Chạy tuần tự các task đủ dependency; model/reasoning được chốt theo cấu hình vai trò khi lưu plan.
- Tester → Reviewer độc lập → Human final review; vòng sửa lỗi có giới hạn, cần người chỉ hướng khi nhiều nhánh không xác định được nơi sửa.
- Pause, Resume, Request cancel; quản lý request phê duyệt/quyền/câu hỏi từ Codex.
- Token, trạng thái, thread/turn, lịch sử controller và checkpoint riêng.
- Overview, Workflow, Activity và Research Team tiếp tục hoạt động như trước.

## Nguồn thật và giới hạn

Autopilot dùng Codex App Server qua stdio, không dùng dữ liệu mock để điều khiển. Đã xác minh initialize/model-list trên Codex CLI 0.153.4: Windows, 4 model. Chưa xác minh một workflow suy luận thật từ đầu tới cuối. Kiểm thử protocol dùng fixture để kiểm tra lỗi và race condition.

Dependency map hiện là mẫu/chỉnh tay, chưa tự sinh bằng AI. Chạy một task mỗi lần trong workspace chung; chưa có thực thi song song/worktree riêng. Giới hạn token dựa trên số liệu nhận được, không phải giới hạn thanh toán tuyệt đối. App cần mở để chạy; không có dịch vụ nền. Cancel không hoàn tác file đã sửa.

State: `%LOCALAPPDATA%\AgentControlCenter\autopilot.json`. Monitor: `history.sqlite3`. Không sửa cấu hình Codex toàn cục trong quá trình triển khai tính năng này.

## Dev/build

Trong thư mục source `outputs/Agent-Control-Center`:

```powershell
npm run tauri dev
npm run test:frontend
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml --lib --offline
npm run tauri build
```

Cần Node.js, Rust MSVC, C++ Build Tools/Windows SDK, WebView2 và Codex CLI đã đăng nhập.

## Xác minh bản build ngày 2026-10-18

Bản EXE và bộ cài đã build thành công từ source cuối. Tester độc lập: 52/52 kiểm thử Rust (gồm 14 transport), 13/13 frontend, typecheck và production build đạt. Reviewer đã duyệt, không còn phát hiện cần xử lý. Đã khôi phục C++ Build Tools và Windows SDK. App mở được, accessibility tree xác nhận màn hình monitor và mục Autopilot V2; chưa thử thao tác Autopilot vì Windows đang khóa. Chưa chạy workflow suy luận thật từ đầu tới cuối.


## SHA-256

- Agent-Control-Center-0.2.0-x64.exe: 16AA348C40C2685AC1A13AFFD28AF78231266BAED9604FCAEDDB36C155FCB7A8
- Agent-Control-Center-0.2.0-x64-setup.exe: A17E01840BD5C67C2D9EA1904D9478646C2224F48FFA1DB53F61905A8904F054

