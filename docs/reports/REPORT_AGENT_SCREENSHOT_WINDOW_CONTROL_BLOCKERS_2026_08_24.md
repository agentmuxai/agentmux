# REPORT: Blockers hit controlling/screenshotting another AgentMux window, live session

**Date:** 2026-08-24
**Author:** Claude (agent session), first-person account
**Context:** While iterating on a composer-strip layout fix, I needed to (a) launch a `task dev` build of AgentMux itself to test the change, (b) identify/rename its window so the user could reliably find it among several open AgentMux windows, and (c) screenshot it myself to verify visual results before asking the user to re-check. Every one of these turned out to be harder than expected. This report documents exactly what happened and why, grounded in a follow-up read of the actual `agentmux-mcp`/`agentmux-srv`/`agentmux-cef` source. It's the input to `SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md`.

---

## 1. `CaptureWindow` matched an unrelated, unrelated-agent window before finding mine

**What happened:** With three AgentMux windows open (my own session, a stale-branch `task dev` build I'd just launched, and — unbeknownst to me — a completely unrelated pre-existing AgentMux window running a different agent's live session), I called `CaptureWindow(title_contains: "AgentMux")` to find my dev build. It silently returned index 0: a screenshot of the *unrelated* window, showing another agent's live conversation — GitHub PAT/credential handling, repo names, account details. Not something I intended to look at, and not logged as ambiguous.

**Why:** `CaptureWindow`'s docstring states *"The error message lists all matching AgentMux window titles when this is ambiguous"* — but in the actual implementation (`agentmux-mcp/src/main.rs:940-1044`, `capture_window_impl`), an ambiguous match (multiple windows matching `title_contains`) does **not** surface the candidate list; it silently captures whichever window sorts first at `index` (default 0). I only discovered there were 3 matches by manually trying `index: 1`, `index: 2` in sequence — guesswork, with a real chance of repeatedly hitting the wrong (and in this case, sensitive) window on each guess.

**Severity:** real — this is a documented behavior gap between the tool's own docstring and its implementation, and it caused an actual unintended exposure to another agent's live session content, which I had to flag to the user mid-task.

## 2. No way to enumerate candidate windows before calling CaptureWindow

**What happened:** To find the *right* index, I had no option but to call `CaptureWindow` itself (index 0, 1, 2, …) and inspect the resulting image each time — spending real capture calls (each one audit-logged, each one producing an image file, each one a chance of hitting a wrong/sensitive window) just to discover what windows exist.

**Why:** There's no read-only "list AgentMux windows across instances" tool. `Layout` returns window/tab/workspace structure, but only for the *calling* agent's own `agentmux-srv` instance (confirmed: it's a local IPC call against the caller's own state, not a cross-instance query). `CaptureWindow` is the only tool that reaches outside the caller's own instance at all, and it's screenshot-or-nothing — there's no cheap, non-image-producing "just tell me what's out there" step.

## 3. A legitimately-running window screenshotted as solid black, twice, with no explanation

**What happened:** After confirming (via `tasklist`/process enumeration) that my dev build's launcher + CEF processes were genuinely running, `CaptureWindow` still returned a fully black PNG — both immediately after launch and again ~30s later. I couldn't tell if this meant "not rendered yet, try again" or "something is actually broken."

**Why (partially explained, partially open):** `capture_window_impl` calls `xcap`'s `window.capture_image()` with no retry and no post-capture sanity check (e.g. detecting a single-color frame) — it returns whatever the OS compositor happened to hand back at that instant. For a freshly-created CEF window that hasn't painted its first frame, or one occluded/minimized/off the visible desktop, a black or blank capture is a completely plausible outcome, but the tool gives the caller no way to distinguish "really nothing to see yet" from "capture is fine, the window itself is genuinely rendering black" from "capture mechanism failed silently."

## 4. `SetName` cannot rename a window in a different `agentmux-srv` instance — confirmed structural, not a missing parameter

**What happened:** The user asked me to rename the dev-build window. `SetName(target: "window", ...)` only accepts a `target_id` resolved against the *caller's own* `Layout`/`WhoAmI` — passing anything else fails, and the dev-build window (a separate `task dev` process, its own `agentmux-srv` sidecar) never appears in my own `Layout` query at all.

**Why:** Confirmed at every layer this is load-bearing, not an oversight. `WindowNameRequest` (`agentmux-common/src/api_types.rs:258-264`) has exactly three fields (`block_id`, `name`, `window_id`) — no PID, no HWND, no cross-instance address. The MCP tool POSTs to `AGENTMUX_LOCAL_URL`, which is fixed once at MCP-process-start to *this* agent's own sidecar and never repointed (`docs/reports/REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md` §3.4). A separate `task dev` instance has its own sidecar, own auth key, own window IDs — reachable by nothing in the current tool surface. This exact limitation was already identified, by name, as "Option B" in a pre-`SetName` design doc (`docs/analysis/REPORT_AGENT_WINDOW_NAMING_2026_06_17.md` §4) and deliberately not built. I wasn't hitting a bug; I was hitting a documented, intentional boundary — but the tool gives no indication *why* the rename fails beyond "target_id required," so I spent real effort rediscovering a limitation that was already known.

## 5. Dropping to raw Win32 `SetWindowText` "worked" then silently stopped working, with a genuinely confusing failure mode

**What happened:** With no supported path, I wrote a throwaway PowerShell script using `user32.dll` P/Invoke (`EnumWindows`/`GetWindowThreadProcessId`/`SetWindowText`) to rename the dev-build window directly by PID. It worked — `Get-Process`/`Get-CimInstance` confirmed the OS-level title changed. Minutes later, `CaptureWindow` couldn't find the window under *any* substring I tried, including ones that should still have matched (the new title, and even "AgentMux" itself, which the new title still contained as a substring).

**Why:** This is the most interesting finding from the follow-up research, and it revises my own live theory at the time (I'd guessed `CaptureWindow` used some kind of stale internal registry — it doesn't; `capture_window_impl` does a fresh `xcap::Window::all()` live OS query on every call, confirmed by reading it directly). The real mechanism: AgentMux's own frontend continuously **overwrites its own OS window title** via a reactive effect (`installWindowTitleEffect`, `frontend/app-init.ts:811-886`) that recomputes and writes `document.title` — which flows to a real `SetWindowTextW` call in `agentmux-cef/src/client/display.rs` (`on_title_change`) — on *any* change to: the active tab, the window's own `window:displayname` meta, its assigned workspace's name, **or its position in the cross-window `openWindowEntriesAtom` list** (which shifts whenever *any* AgentMux window on the machine opens or closes, not just this one). During the same window of time I was renaming this window, I was also killing and relaunching other dev-build process trees — each open/close is exactly the kind of event that re-triggers this reactive effect and overwrites the title back to AgentMux's own live-computed value, independent of what I'd just set it to externally. An externally-forced OS-level rename on this app is fighting a live process that will eventually win — it isn't durable, and the failure mode (silent, delayed, no error) makes it look like a `CaptureWindow` bug rather than what it actually is: a race against the app's own title-management system.

## 6. Launching a long-running GUI dev-server process was unreliable across three different mechanisms

**What happened, three attempts:**
1. `mcp__agentmux__Shell(cmd: "npm run dev", ...)` — died with exit code 201 after ~558-559 lines of output, twice in a row, at reproducibly the same point in the build/startup sequence. No error message surfaced to me beyond the bare exit code.
2. Plain `Bash(cmd: "npm run dev > log 2>&1 &"; disown)` — the backgrounded job didn't survive at all; the redirected log file stayed empty, no process ever appeared in `tasklist`. Silent no-op.
3. `Bash(cmd: "npm run dev > log 2>&1", run_in_background: true)` — worked. The app built, launched, and ran successfully for ~10 minutes (confirmed via live log output showing full CEF/frontend startup) — then was killed by an external SIGTERM I never issued.

**Why (mixed — two different systems, don't conflate them):** Follow-up research into `agentmux-srv`'s own background-shell engine (`ShellNodeRunner`, `agentmux-srv/src/backend/shell_node.rs` — the real implementation behind the `Shell`/`ShellStatus` MCP tools) found **no idle-timeout or max-runtime cap for it at all**. The only lifecycle-ending paths are natural process exit, an explicit `ShellStop`, or the whole `agentmux-srv` instance shutting down. The interactive-agent-pane PTY subsystem (`ShellController`) *does* have a watchdog with idle/wall-clock caps (`agentmux-srv/src/backend/blockcontroller/watchdog.rs`), but it explicitly skips anything that isn't `is_agent_pane` — `Shell`-tool processes aren't controllers at all and are invisible to that watchdog. So attempt #1's exit-201 failure is **not** explained by any lifetime cap in AgentMux's own code — its root cause is still unknown; flagging as an open item below. Attempt #3's SIGTERM-after-10-minutes is a **different system entirely** — the generic `Bash` tool's own background-execution mechanism (part of my own agent harness, not `mcp__agentmux__Shell`/`ShellNodeRunner`) — so whatever imposed that cutoff is out of scope for an AgentMux-side fix; noted here only so the two failures aren't mistaken for the same root cause.

## 7. Cleaning up an orphaned process tree after the SIGTERM required manual PID archaeology

**What happened:** After the attempt-#3 process got killed externally, its child processes (launcher + `agentmux-srv` + several `agentmux-cef.exe` renderer/gpu/utility children) survived as orphans and held a file lock on `agentmux-launcher.exe`, silently blocking every subsequent `npm run dev` attempt with `The process cannot access the file because it is being used by another process` — no indication *what* was holding the lock. I had to manually reconstruct the process tree via `Get-CimInstance Win32_Process` (parent/child + command-line matching on the dev build's own `dist/cef-dev` path, to avoid mis-identifying or killing sibling AgentMux instances that happen to share process *names*), then `taskkill /PID <root> /T /F` the correct root.

**Why:** Confirmed `ShellNodeRunner` does have a proper, PID/process-group-scoped `kill_tree` for `ShellStop` (`shell_node.rs:176-186`) — the building block for clean teardown exists — but only reachable via the `shell_id` returned at launch time. If that id is lost (session interruption, or in this case a process that outlived the tool call that spawned it via a *different* backgrounding mechanism than `ShellNodeRunner`), there's no tool to discover "what did I spawn earlier that might still be running" independent of holding onto the original id.

## 8. Unrelated but directly blocking: Windows-style CLI flags (`/PID`, `/T`, `/F`) got mangled by MSYS path conversion

**What happened:** `taskkill /PID 68108 /T /F` failed with `Invalid argument/option - 'C:/Program Files/Git/PID'` — Git Bash's automatic Unix-path conversion rewrote `/PID` as if it were a filesystem path. Needed `MSYS_NO_PATHCONV=1` or routing through `cmd.exe /C "..."` explicitly to work around it. Not screenshot/window-control specific, but it directly blocked the cleanup in #7, and cost several failed attempts before I found the right invocation.

---

## Summary table

| # | Blocker | Root cause confirmed? | In scope for the follow-up spec? |
|---|---|---|---|
| 1 | Ambiguous match silently returns index 0, no candidate list | Yes — docstring/implementation gap | Yes |
| 2 | No cheap window-discovery/list tool | Yes — doesn't exist | Yes |
| 3 | Black screenshot, no retry/explanation | Partially — no retry logic exists; can't confirm compositor-level cause | Yes (retry/flag) |
| 4 | SetName can't reach a foreign instance | Yes — confirmed structural, already documented as deliberate elsewhere | Yes (propose a real solution, not a workaround) |
| 5 | Raw external rename gets silently overwritten | Yes — AgentMux's own reactive title effect wins the race | Yes (via #4's real fix, not by hardening the workaround) |
| 6 | `Shell`-tool GUI process died at exit 201 | **No — open, unresolved** | Flag as investigation item, not a proposed fix |
| 6b | Bash-harness background job SIGTERM'd after ~10min | Out of scope — different system (agent harness, not AgentMux) | No |
| 7 | Orphan cleanup needed manual PID archaeology | Yes — no discovery path once `shell_id` is lost | Yes |
| 8 | MSYS path-mangling on Windows-style flags | Yes — known Git-Bash behavior | Out of scope (shell environment quirk, not an AgentMux tool) |
