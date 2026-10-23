# Agent Control Center 0.3.0

## Thay đổi

- Activity log nhóm các sự kiện liên tiếp cùng role, thread và loại; mở rộng để xem từng bản ghi. Các lỗi và mốc trạng thái khác nhau vẫn hiển thị riêng.
- Cards và Workflow phân biệt trạng thái mới được quan sát, trạng thái cũ và không rõ thời điểm. Bộ đếm Working chỉ tính bằng chứng trong hai phút gần nhất.
- Thời điểm quan sát trạng thái tách khỏi thời điểm có hoạt động bất kỳ, tránh biến trạng thái cũ thành “đang chạy” chỉ vì có thêm log.
- Autopilot đọc nội dung event thật từ App Server để hiển thị lệnh, file, thông điệp và kết quả; gộp các đoạn streaming thành bản tóm tắt có giới hạn.

## Nguồn dữ liệu và giới hạn

Overview, Workflow và Activity đọc session JSONL thật của Codex, lưu lịch sử monitor bằng SQLite. Autopilot sử dụng kết nối App Server riêng cho các task do ứng dụng quản lý. Hai nguồn không phải cùng một kết nối.

“Last observed” không có nghĩa agent đã dừng: lệnh chạy lâu có thể chưa tạo thêm bằng chứng. Event streaming là bản xem trước có giới hạn; không thay thế toàn bộ transcript. Lịch sử controller có giới hạn dung lượng.

## Chạy từ source

Source: thư mục `Agent-Control-Center` cạnh tài liệu này.

```powershell
npm run tauri dev
npm run test:frontend
npm run typecheck
npm run tauri build
```

Đóng bản ứng dụng cũ trước khi mở EXE mới để tránh hai controller cùng sử dụng một plan.

## Xác minh

Ngày xác minh: 20/10/2026.

- Tester độc lập: 65/65 Rust, 21/21 frontend, typecheck và production build đạt.
- Reviewer độc lập: duyệt tích hợp sau khi sửa thời lượng terminal dùng đúng thời điểm trạng thái thay vì thời điểm hoạt động bất kỳ.
- Tauri release EXE và bộ cài NSIS x64 build thành công. Có cảnh báo dead-code cho `mark_turn_started`; không chặn build.
- Smoke test native với dữ liệu thật: Overview kết nối session local; nhãn recent/last observed/time unknown hiển thị; Workflow đồng bộ; mở nhóm Activity thấy đủ 8 bản ghi cùng thread, thời gian và ID; light/dark hiển thị đúng sau render.
- Autopilot kết nối App Server thành công, đọc lại plan cũ Completed 5/5, không task nào chạy. Không chạy workflow/model mới để thử bản này; bộ chuẩn hóa streaming được xác minh bằng kiểm thử tự động.

Các artifact:

- `Agent-Control-Center-0.3.0-x64.exe`: EXE chạy trực tiếp.
- `Agent-Control-Center-0.3.0-x64-setup.exe`: bộ cài Windows, đã build nhưng không chạy cài đặt trong lượt này.

SHA256:

```text
3C29D9E5526286B266C5A93D05018EAB6142A77EA1CC75944303E5E2CEAE1C3B  Agent-Control-Center-0.3.0-x64.exe
B330E7DBDBC30C61B696BC8E587C104813B3B800D7DD513A88295C361EAB19C1  Agent-Control-Center-0.3.0-x64-setup.exe
```
