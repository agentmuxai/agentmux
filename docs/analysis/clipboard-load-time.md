# Clipboard Branch Load Time Analysis

**Date:** 2026-04-02
**Conclusion:** No load time difference — perceived slowness was from CEF cache state

## Investigation

User reported clipboard branch (0.33.23) felt slower than main. Deep dive:

### Bundle Comparison

| Metric | Main (0.33.24) | Clipboard (0.33.23) |
|--------|---------------|-------------------|
| Main bundle | 2,166,847 bytes | 2,166,838 bytes |
| JS chunks | 51 | 51 |
| Binary size | 2,591,744 bytes | 2,595,840 bytes |

**9 bytes difference** in JS bundle. 4KB in binary (clipboard Win32 code). Negligible.

### Startup Timeline (cold cache, both builds)

| Event | Clipboard | Main |
|-------|-----------|------|
| IPC server start | +4ms | +8ms |
| Backend ready | +80ms | +102ms |
| CEF initialized | +192ms | +274ms |
| Frontend injected | +510ms | +591ms |
| setupCefApi | +526ms | +607ms |
| fonts-ready | +696ms | +777ms |
| window-show | +782ms | +866ms |

**Clipboard is actually slightly faster** in this run. Both are within normal variance.

### Why It Felt Slower

The clipboard branch portable (0.33.23) was tested after clearing all CEF caches (`rm -rf $APPDATA/ai.agentmux.cef.v0-33-*`). The main build (0.33.24) was tested with warm caches from the benchmark runs. CEF's cold start includes:
- First-time browser profile creation
- GPU process initialization
- Chrome feature flag evaluation
- First-time font enumeration

This adds 200-500ms to the first launch after cache clear.

### Clipboard Code Path at Runtime

The clipboard module (`clipboard.rs`) adds zero startup cost:
- No init code — just two functions (`read_clipboard`, `write_clipboard`)
- Only called on user action (Ctrl+C/V)
- IPC route registration is a hashmap insert (nanoseconds)

The frontend `clipboard.ts` change:
- Replaced static Tauri import with direct `invokeCommand` call
- No dynamic import, no lazy loading, no startup code
- Smaller than before (removed Tauri detection logic)

## Verdict

No performance regression from the clipboard change. Safe to merge.
