# Agent Control Center 0.1.5

Windows x64 — 2026-10-09.

## Chạy bản mới

- `Agent-Control-Center-0.1.5-x64-setup.exe`: bộ cài.
- `Agent-Control-Center-0.1.5-x64.exe`: chạy trực tiếp với WebView2.
- Source: thư mục `Agent-Control-Center` cạnh báo cáo.

Đóng phiên bản cũ trước khi mở bản mới. Các bản phát hành trước vẫn được giữ lại.

## Sửa trạng thái subagent

- Tên task tùy ý được liên kết với role từ thông tin tạo agent; không yêu cầu task phải có tên `coder` hoặc `tester`.
- Các task cùng role có định danh riêng. Trong nhóm phiên cha mới nhất, thẻ ưu tiên task có trạng thái Working đã quan sát và hiển thị số task khi có nhiều task.
- Phiên Working cũ từ phiên cha khác không lấn át trạng thái mới. Lần giao việc tiếp tìm đúng task, đặt lại thời điểm bắt đầu và ghi rõ trạng thái chưa được xác nhận.
- Chỉ kết quả có `call_id` khớp lệnh `list_agents` của cùng phiên cha mới cập nhật lifecycle được suy ra từ phiên cha.
- Khi log bị ghi lại từ đầu, các liên kết lệnh gọi cũ được loại bỏ để tránh cập nhật nhầm.
- Thẻ Overview và Workflow có **Last observed**, bằng chứng gần nhất và nhãn thời gian được quan sát. Waiting/Idle không còn mang bộ đếm runtime như đang chạy.

## Giới hạn cần biết

Ứng dụng vẫn đọc Codex JSONL local, chưa có kết nối App Server đảm bảo trạng thái thực thi liên tục. Với subagent chỉ được ghi qua phiên cha, dispatch/followup không chứng minh agent đang chạy: cần bản trạng thái `list_agents` đã đối chiếu hoặc log lifecycle trực tiếp của phiên con. Khi thiếu dữ liệu này, Waiting là **chưa có xác nhận mới**, không có nghĩa chắc chắn agent không làm việc.

Trạng thái Working/Done là trạng thái quan sát gần nhất, có thể cũ nếu không có sự kiện tiếp theo. Xem **Last observed** và **Activity log** để biết thời điểm cùng nguồn cập nhật. Mỗi role vẫn có một thẻ; đây chưa phải danh sách đầy đủ mọi task trên máy. Research Team tiếp tục giữ các bước phê duyệt thủ công.

## Kiểm chứng

- Tester độc lập: 19 kiểm thử Rust, 5 kiểm thử frontend, typecheck và production frontend build đạt.
- Kiểm thử riêng sau bản sửa cuối: truncation cleanup đạt; xác nhận `call_id` cũ không thể cập nhật task sau khi log được ghi lại.
- Reviewer độc lập đã duyệt bản cuối.
- `npm run tauri build` thành công, tạo EXE và NSIS installer 0.1.5.
- Kiểm tra GUI với dữ liệu Codex thật ngày 09/10: Workflow hiển thị Reviewer Working; Activity ghi Coder/Tester/Reviewer Working lúc 16:20:56, Reviewer Working lúc 16:22:50 và Done lúc 16:23:44. Đây là bằng chứng chuyển trạng thái từ các bản `list_agents` thực tế, không dùng dữ liệu mock.
- Không thay đổi cấu hình model Codex trong lần sửa này. Chưa chạy trình cài đặt NSIS trên máy sạch.

## File và lệnh chính

`src-tauri/src/lib.rs`: nhận diện task, lifecycle, chọn thẻ, kiểm thử Rust.

`src/agent-observation.js`, `src/main.tsx`, `src/WorkflowPage.tsx`: bằng chứng và thời điểm quan sát.

```powershell
npm run tauri dev
npm run test:frontend
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml --lib --offline
npm run tauri build
```

## SHA-256

Installer: `C217669FB3E657A9F2E2113C0A59234523DC1DD030A7F4CC98C91BD3AEC2D864`

Standalone: `DD3BE5C70B8F3D0EAE95B56F6BEF5F93458C141FD99ACA0C8AA266B45451692F`
