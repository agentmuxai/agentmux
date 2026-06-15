---
type: patch
---

fix(cef): add `.0` before the `HWND -> *mut c_void` cast in app.rs (unbreak the Windows build; windows 0.57 `HWND` is a non-primitive struct).
