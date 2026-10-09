# Agent Control Center 0.1.4

Windows x64 release — 2026-10-08.

## Chạy ứng dụng

- `Agent-Control-Center-0.1.4-x64-setup.exe`: bộ cài Windows.
- `Agent-Control-Center-0.1.4-x64.exe`: chạy trực tiếp; cần WebView2.
- Source: thư mục `Agent-Control-Center` cạnh báo cáo này.

Đóng phiên bản cũ trước khi mở bản mới. Các bản phát hành 0.1.2 và 0.1.3 vẫn được giữ lại.

## Workflow

Chọn **Workflow** trên sidebar. Software Team hiển thị Planner → Coder/Mobile Dev → Tester → Reviewer. Bấm từng vai trò để xem trạng thái, nhiệm vụ, model, thời gian, token, workspace, mã phiên và hoạt động gần nhất khi có dữ liệu. **Open activity log** mở lịch sử với bộ lọc vai trò tương ứng.

Research Team vẫn giữ thứ tự nghiên cứu và các bước phê duyệt thủ công. Sơ đồ là trạng thái mới nhất theo vai trò; các vai trò có thể thuộc nhiều phiên/workspace khác nhau. Mũi tên không xác nhận việc bàn giao hoặc phê duyệt đã xảy ra.

## Activity log

- Truy vấn lịch sử SQLite thật; không giới hạn ở số dòng trên Overview.
- Lọc team, vai trò, loại sự kiện; tìm chuỗi trong nội dung tóm tắt.
- Hiển thị ngày giờ đầy đủ; bấm dòng để xem mã phiên, record ID và nội dung đã lưu.
- Mỗi trang 50 dòng. **Load older events** tải thêm và giữ chế độ duyệt lịch sử; **Refresh** quay về các bản ghi mới nhất.
- Cập nhật nền giữ danh sách hiển thị. Các truy vấn cũ bị vô hiệu khi đổi bộ lọc; truy vấn refresh nền được tuần tự hóa.
- Thứ tự là bản ghi mới được lưu trước, không phải sắp xếp theo thời điểm sự kiện gốc. Nội dung là tóm tắt đã lưu, tối đa 180 ký tự, không phải toàn bộ transcript.

## Kết nối và giới hạn

Dữ liệu vẫn đến từ Codex session JSONL local, cập nhật khoảng 2 giây; lịch sử ở `%LOCALAPPDATA%/AgentControlCenter/history.sqlite3`. Chưa kết nối trực tiếp transport App Server. Không có nút start/stop/steer. Model Settings của 0.1.3 được giữ nguyên và chỉ ghi cấu hình khi người dùng bấm Save role.

Thông tin model/token/trạng thái có thể thiếu trong log; ứng dụng hiển thị unavailable hoặc trạng thái quan sát được gần nhất. Các sự kiện lịch sử chỉ chuẩn hóa mã phiên khi mapping phiên tương ứng đã được nạp.

## Kiểm chứng

- Tester độc lập: 17 kiểm thử Rust và 3 kiểm thử frontend đạt; typecheck đạt.
- Reviewer độc lập: các lỗi liên kết phiên, bộ lọc Overview ảnh hưởng Workflow, refresh chồng lấp và trạng thái nút bị kẹt đã được rà soát và sửa.
- Build cuối `npm run tauri build` thành công, tạo EXE và bộ cài NSIS.
- GUI smoke với log Codex thật: kết nối local, Workflow Planner và related events, chuyển sang Activity lọc Planner, tải từ 50 lên 100 dòng, trạng thái duyệt lịch sử cũ. Lỗi cuộn về đầu khi refresh đã được tái hiện và bản sửa đã được kiểm tra trực tiếp.
- Hai bản sửa cuối về tuần tự refresh và reset trạng thái khi đổi bộ lọc được kiểm tra bằng test/typecheck/review; chưa chạy lại toàn bộ chuỗi GUI sau các thay đổi này. Chưa kiểm thử cài đặt NSIS trên máy sạch.
- Không dùng Model Settings để ghi cấu hình Codex thật. Bốn file cấu hình agent không đổi so với baseline; config toàn cục khác baseline giữa hai ngày làm việc và được giữ nguyên, không ghi đè.

## Dev/build

Trong thư mục source:

```powershell
npm install
npm run tauri dev
npm run test:frontend
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml --lib --offline
npm run tauri build
```

File chính: `src/WorkflowPage.tsx`, `src/ActivityPage.tsx`, `src/activity-logic.js`, `src/workflow-activity.css`, `src/main.tsx`, `src-tauri/src/lib.rs`, `tests/activity-logic.test.mjs`.
