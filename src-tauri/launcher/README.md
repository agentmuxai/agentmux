# AgentMux Launcher

This is a tiny C++ launcher that shows the splash screen immediately when double-clicked, then launches the main AgentMux executable.

## Building

### Option 1: Visual Studio (MSVC)
```cmd
"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cl /O2 /MT /EHsc launcher.cpp user32.lib gdi32.lib /Fe:AgentMux.exe
```

### Option 2: MinGW-w64
```bash
g++ -O2 -static -mwindows launcher.cpp -o AgentMux.exe -lgdi32 -luser32
```

### Option 3: Build from Rust (if no C++ compiler)
Use the provided `launcher-rust` alternative (slower startup but doesn't require C++ compiler).

## Deployment

1. Build launcher → `AgentMux.exe` (~20KB)
2. Rename Rust executable → `agentmux.exe` → `agentmux-main.exe`
3. Copy together:
   - `AgentMux.exe` (launcher - what user clicks)
   - `agentmux-main.exe` (real app)
   - `splash.bmp` (400x400 24-bit BMP)

User double-clicks `AgentMux.exe` → splash shows instantly → launches `agentmux-main.exe`
