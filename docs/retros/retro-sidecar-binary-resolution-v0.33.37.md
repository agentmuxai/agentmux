# Retro: Portable v0.33.37 Failed to Launch

**Date:** 2026-04-04
**Severity:** P0 — portable completely broken
**Root cause:** Double `runtime/` path in sidecar binary resolution
**Fix time:** ~5 min after diagnosis

## What Happened

After renaming `agentmuxsrv-rs` → `agentmux-srv` and adding versioned binary names,
the portable build completed successfully but double-clicking `agentmux.exe` showed nothing.

## Root Cause

The `resolve_backend_binary()` function in `agentmux-cef/src/sidecar.rs` searched for
the backend binary using `exe_dir.join("runtime").join(...)`.

**The problem:** In the portable layout, the CEF host binary is already inside `runtime/`:
```
portable/
  agentmux.exe           ← launcher
  runtime/
    agentmux-cef.exe     ← CEF host (this is current_exe())
    agentmux-srv-...exe  ← backend (what we're looking for)
```

So `exe_dir` = `portable/runtime/`, and `exe_dir.join("runtime")` = `portable/runtime/runtime/`
which obviously doesn't exist.

The old code had the same structure but used `exe_dir.join("runtime")` as one of several
fallback paths, and worked because it also tried `exe_dir.join(format!("{}.x64{}", ...))` 
which searched in the same directory. The new code put the versioned search in `exe_dir.join("runtime")`
as the FIRST candidate without an equivalent same-directory search.

## Fix

Search for the backend binary in `exe_dir` (same directory as CEF host) first, then fall back
to parent directory patterns for dev mode:

1. `exe_dir/{name}-{version}-{os}.{arch}.exe` — portable (same dir as CEF host)
2. `exe_dir/{name}.exe` — dev mode (cargo build output is adjacent)
3. `exe_dir/../dist/bin/{name}-{version}-{os}.{arch}.exe` — workspace dev layout

## Lesson

- **Always test the portable** before pushing a PR that touches sidecar resolution
- The launcher and CEF host are in different directories — `current_exe()` for the
  CEF host is inside `runtime/`, not at the portable root
- Error messages should include the actual `exe_dir` path, not just the searched paths
- The old code worked by accident (one of its many fallbacks happened to search the right dir)
