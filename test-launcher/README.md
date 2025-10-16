# AgentMux Launcher Test

This directory contains the launcher setup for testing instant splash screen.

## Files
- **AgentMux.exe** (214KB) - Small launcher that shows splash immediately
- **agentmux-main.exe** (19MB) - Real AgentMux application
- **splash.bmp** (469KB) - Splash screen image

## How to Test
1. **Double-click `AgentMux.exe`**
2. You should see the splash screen appear **immediately** (within 50-200ms)
3. Splash stays visible while main app loads
4. Splash closes automatically when main window appears

## Expected Results
- ✅ Splash appears instantly after double-click
- ✅ Purple screen with "AgentMux Loading..." text
- ✅ Smooth transition to main window
- ✅ No black square or delay

## What Changed
- **Before**: Main executable showed splash after Rust initialization (too late)
- **After**: Tiny launcher shows splash immediately, then launches main executable

## Architecture
```
User double-clicks AgentMux.exe (214KB)
  ↓ <50ms
Splash screen appears (purple + loading text)
  ↓
Launcher spawns agentmux-main.exe in background
  ↓
Main app initializes (Rust + Tauri + WebView2)
  ↓
Main app creates signal file when ready
  ↓
Launcher detects signal and closes splash
  ↓
Main window appears
```

## Deployment
For production, bundle these 3 files together:
- Rename `AgentMux.exe` to what user clicks
- Keep `agentmux-main.exe` and `splash.bmp` in same directory
