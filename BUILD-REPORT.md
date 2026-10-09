# Agent Control Center 0.1.2 — Windows x64

Ngày bàn giao: 30/09/2026.

## Sử dụng

- `Agent-Control-Center-0.1.2-x64-setup.exe`: bộ cài Windows.
- `Agent-Control-Center-0.1.2-x64.exe`: mở trực tiếp trên máy có WebView2.
- Source đầy đủ ở thư mục `Agent-Control-Center` cùng cấp với tài liệu này.

Ứng dụng tự đọc thư mục `%USERPROFILE%\.codex\sessions`, hoặc `CODEX_HOME\sessions` nếu biến môi trường đó được đặt. Lịch sử hoạt động lưu tại `%LOCALAPPDATA%\AgentControlCenter\history.sqlite3`. Ứng dụng chỉ giám sát; không gửi lệnh điều khiển agent.

## Kiểm tra

- `cargo test`: 11 kiểm thử đạt, gồm nhận diện vai trò, lọc guardian, trạng thái agent con, token, đọc lại log, nạp lại lịch sử SQLite và tiếp tục theo dõi session cũ.
- `npm run typecheck`: đạt.
- `npm run build`: đạt.
- Reviewer độc lập đã rà soát source cuối và không còn finding cần sửa.
- Bộ cài được tạo bằng `npm run tauri build` với Tauri/NSIS cho Windows x64.
- Đã mở chính file EXE bàn giao trên Windows: cửa sổ phản hồi, nguồn log local kết nối, activity log cập nhật; Coder, Tester và Reviewer hiển thị Done từ kết quả trạng thái thực tế. Nhãn V1.2 và tổng token toàn bộ session hiển thị đúng. Giao diện sáng/tối và chuyển Software/Research đã được kiểm tra trong quá trình hoàn thiện.

SHA-256 của file bàn giao:

```text
Agent-Control-Center-0.1.2-x64.exe
2F4D93E4F542A7063FF91ED9DBA9C38E74429405976FF106C79F1198E562AF82

Agent-Control-Center-0.1.2-x64-setup.exe
6CE4E1D15F94830F9548E4F5FFE8FDABD42F6BF3C86B5887E65D1974A286F1B0
```

## Phạm vi dữ liệu

- Nguồn là log local thật của Codex, cập nhật mỗi hai giây; chưa kết nối trực tiếp vào transport của Codex Desktop App Server.
- Khi khởi động, app đọc 80 log session gần nhất, rồi tiếp tục theo dõi những file đã nhận diện.
- Các thẻ ưu tiên session có vai trò rõ ràng. Nếu không có session riêng, app dùng bản ghi `spawn_agent` và kết quả `list_agents` có cấu trúc để nhận diện agent con. Trạng thái chỉ phản ánh sự kiện gần nhất có trong nguồn log.
- Nội dung mã hóa, model và token không thể quy về một agent được ghi rõ là không có dữ liệu; không hiển thị chuỗi mã hóa hoặc gán token của luồng chung cho Planner.
- `OBSERVED TOKENS` là tổng các bản ghi usage đã đọc từ tất cả session được theo dõi, không phải riêng đội đang chọn và không phải số dư hạn mức tài khoản.
- Software Team và Research Team được giữ nguyên. Một vai trò có nhiều session ở nhiều project sẽ hiển thị session gần nhất; V1.2 chưa có bộ lọc project.

## Chạy từ source

Trong thư mục `Agent-Control-Center`:

```powershell
npm install
npm run tauri dev
```

Tạo lại bộ cài:

```powershell
npm run tauri build
```

Máy build cần Node.js, Rust MSVC, Microsoft C++ Build Tools và WebView2. Người dùng mở EXE không cần Node.js hoặc Rust.
