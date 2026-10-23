# Agent Control Center 0.2.1

## Bản vá

- Sửa nút gửi chỉ dẫn tại structured human gate: **Continue with direction** gửi feedback và chạy lại task trong giới hạn số lần thử.
- **Reject and end run** là hành động từ chối riêng, được ghi rõ. Human final gate vẫn có Approve và Retry.
- Giữ nguyên kế hoạch và lịch sử đã lưu. Bản vá không tự phục hồi hoặc chạy lại plan Failed.

## Kiểm tra dữ liệu thật

Sau khi mở khóa Windows, kiểm tra trực tiếp bản 0.2.0 cho thấy App Server ready, 4 model và một lượt Planner do người dùng khởi chạy có khoảng 25K token. Plan đã Failed tại sự kiện Human gate rejected; đây là nơi phát hiện nút Send direction gửi nhầm hành động từ chối.

Chưa xác minh workflow từ Planner tới final review chạy thành công từ đầu tới cuối. Không chạy thêm lượt model hoặc triển khai project của người dùng khi kiểm tra bản vá.

## Chạy và build

Source: `Agent-Control-Center` cạnh file này. Đóng bản cũ trước khi mở EXE mới để tránh hai controller dùng chung state.

```powershell
npm run tauri dev
npm run test:frontend
npm run typecheck
npm run tauri build
```

Cần Node.js, Rust MSVC, C++ Build Tools/Windows SDK, WebView2 và Codex CLI đã đăng nhập. Các giới hạn Autopilot V2 trong RELEASE-0.2.0.md vẫn áp dụng: map mẫu/chỉnh tay, chạy tuần tự, app cần mở, cancel không hoàn tác file.

Muốn chạy lại plan Failed: chỉnh và lưu revision mới, duyệt rồi Start. Việc này có thể chạy lại task và dùng thêm token; bản vá không tự thực hiện.

## Xác minh ngày 2026-10-18

Tester độc lập: 15/15 frontend tests, typecheck và production build đạt. Regression kiểm tra action model và payload IPC được page sử dụng cho Continue/Reject. Reviewer duyệt sau khi bổ sung regression, không còn phát hiện cần xử lý. Rust không đổi ngoài version; 52/52 Rust tests là kết quả của bản 0.2.0, không chạy lại ở bản vá này.

Build Tauri release và NSIS từ source cuối thành công. Kiểm tra native với dữ liệu thật thực hiện trên 0.2.0; chưa thao tác human gate trên EXE 0.2.1 để tránh chạy lại plan của người dùng. Build còn warning dead_code cho mark_turn_started, không chặn build.

## SHA-256

- Agent-Control-Center-0.2.1-x64.exe: `3AD1ECF38F33C7D14C415012B3DA9CB57E8C4FFB6BC1AF48DA68CED5014EAAEF`
- Agent-Control-Center-0.2.1-x64-setup.exe: `6C4867BE0B1DB0D94B192488E9B251D818C24E6FCCCD782205F1E7B6238A2009`
