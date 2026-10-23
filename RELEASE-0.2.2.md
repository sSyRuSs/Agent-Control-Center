# Agent Control Center 0.2.2 — UI

- Sidebar đứng yên khi nội dung cuộn; sidebar có vùng cuộn riêng khi cửa sổ thấp.
- Nút menu trên thanh đầu để thu gọn/mở rộng. Desktop thu gọn thành thanh biểu tượng 72px.
- Bố cục co giãn theo cửa sổ; nội dung, grids, forms và logs giới hạn chiều rộng.
- Light/dark mode hoàn thiện màu trạng thái; lần đầu theo hệ điều hành, sau đó lưu lựa chọn. Trạng thái sidebar cũng lưu bằng localStorage.

## Xác minh

Tester độc lập: 15/15 frontend tests, typecheck và production build đạt. Reviewer duyệt sau khi sửa độ tương phản nhãn cảnh báo trong light mode. Tauri EXE và NSIS build thành công. Backend không đổi ngoài version; không chạy lại Rust suite.

Kiểm tra Edge trên Vite: 960×640 và 1440×900 không tràn ngang trên Overview; sidebar top=0 trước/sau khi content cuộn 500px; thu gọn còn 72px; theme và trạng thái sidebar giữ sau reload. Đã xem ảnh light/dark. Đây là kiểm tra frontend trong trình duyệt: Tauri IPC không có nên Connection error là dự kiến. Chưa kiểm tra lại native EXE với dữ liệu live ở bản này; không chạy workflow hoặc thay đổi plan của người dùng.

## Sử dụng

Đóng bản cũ trước khi mở Agent-Control-Center-0.2.2-x64.exe hoặc chạy bộ cài tương ứng. Nút menu ở góc trái thanh đầu; nút mặt trời/mặt trăng ở góc phải.

Source nằm trong thư mục Agent-Control-Center cạnh file này. File UI chính: src/main.tsx, src/style.css.

```powershell
npm run tauri dev
npm run test:frontend
npm run typecheck
npm run tauri build
```
