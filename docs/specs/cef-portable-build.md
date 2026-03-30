# Spec: CEF Portable Build Pipeline

**Date:** 2026-03-29
**Status:** Implemented

---

## Portable Layout

```
agentmux-cef-portable/
  agentmux.exe              — 2.5 MB  (launch this)
  README.txt                — instructions
  runtime/
    libcef.dll              — 251 MB  (Chromium 146 engine)
    chrome_elf.dll          — 2.4 MB
    d3dcompiler_47.dll      — 4.6 MB  (WebGL shader compiler)
    icudtl.dat              — 11 MB   (Unicode/ICU data)
    v8_context_snapshot.bin — 700 KB  (V8 startup snapshot)
    chrome_100_percent.pak  — 700 KB  (UI resources @1x)
    chrome_200_percent.pak  — 1.2 MB  (UI resources @2x)
    resources.pak           — 18 MB   (DevTools + Chromium resources)
    agentmuxsrv-rs.x64.exe — 8.6 MB  (backend sidecar)
    locales/
      en-US.pak             — 400 KB  (English only)
    frontend/
      index.html            — Vite-built SPA
      assets/               — JS/CSS bundles
      fonts/                — font files
```

## Size

| Metric | Value |
|--------|-------|
| Uncompressed | 311 MB |
| ZIP compressed | **148 MB** |
| Tauri portable (comparison) | 16 MB |
| Electron typical (comparison) | ~150 MB |

### What was trimmed

| Item | Saved |
|------|-------|
| Locales (220 → 1 file) | 49 MB |
| Vulkan software renderer | 6 MB |

### What can't be trimmed

- `libcef.dll` (251 MB) — prebuilt Chromium, can't shrink without rebuilding from source
- `resources.pak` (18 MB) — contains DevTools frontend, needed for remote debugging
- `icudtl.dat` (11 MB) — Unicode data, required for text rendering

---

## Build Pipeline

### Prerequisites

```bash
# VS-bundled CMake + Ninja (not in PATH by default)
export PATH="/c/Program Files/Microsoft Visual Studio/18/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja:/c/Program Files/Microsoft Visual Studio/18/Community/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin:$PATH"

# CEF SDK path (debug build has the full distribution with libcef_dll source)
export CEF_PATH="/c/Systems/agentmux/target/debug/build/cef-dll-sys-*/out/cef_windows_x86_64"
```

### Steps

```bash
# 1. Build frontend (production)
npx vite build --config vite.config.tauri.ts

# 2. Build CEF host (release)
cargo build --release -p agentmux-cef

# 3. Bundle portable
#    (see Taskfile cef:package task or manual script below)
```

### Manual bundle script

```bash
CEF_SDK="target/debug/build/cef-dll-sys-*/out/cef_windows_x86_64"
DIST="dist/agentmux-cef-portable"
rm -rf "$DIST" && mkdir -p "$DIST/runtime/locales" "$DIST/runtime/frontend"

# Root
cp target/release/agentmux-cef.exe "$DIST/agentmux.exe"

# Runtime
cp $CEF_SDK/{libcef.dll,chrome_elf.dll,d3dcompiler_47.dll,icudtl.dat,v8_context_snapshot.bin} "$DIST/runtime/"
cp $CEF_SDK/*.pak "$DIST/runtime/"
cp $CEF_SDK/locales/en-US.pak "$DIST/runtime/locales/"
cp dist/bin/agentmuxsrv-rs.x64.exe "$DIST/runtime/"
cp -r dist/frontend/* "$DIST/runtime/frontend/"

# ZIP
cd dist && powershell Compress-Archive -Path 'agentmux-cef-portable/*' -DestinationPath 'agentmux-cef-portable.zip'
```

---

## Runtime Detection

`agentmux.exe` auto-detects its environment:

1. **DLL loading:** `SetDllDirectoryW("runtime/")` so Windows finds `libcef.dll`
2. **CEF resources:** `resources_dir_path` + `locales_dir_path` set to `runtime/`
3. **Frontend:** IPC server checks `runtime/frontend/index.html` → serves static files
4. **Sidecar:** Checks `runtime/agentmuxsrv-rs.x64.exe` first
5. **URL:** If frontend exists, loads from `http://127.0.0.1:{ipc_port}/`; otherwise `http://localhost:5173` (Vite dev)

---

## Dev Mode vs Portable

| | Dev Mode | Portable |
|---|---|---|
| Frontend | Vite dev server (localhost:5173) | Static files in runtime/frontend/ |
| CEF binary | target/debug/agentmux-cef.exe | agentmux.exe (release) |
| libcef.dll | Next to exe (flat layout) | runtime/libcef.dll |
| Sidecar | target/debug/agentmuxsrv-rs.exe | runtime/agentmuxsrv-rs.x64.exe |
| Launch | `./agentmux-cef.exe --use-native --url=http://localhost:5173` | `./agentmux.exe --use-native` |
| Hot reload | Yes (Vite HMR) | No |

---

## Launch Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--use-native` | off | Use native Win32 window (required for HTML5 DnD) |
| `--use-alloy-style` | off | Use Alloy runtime style (CEF Views — DnD broken) |
| `--url=URL` | auto-detect | Override frontend URL |
| `--disable-gpu` | off | Software rendering (for GPU issues) |
