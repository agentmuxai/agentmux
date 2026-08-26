# SPEC: Robust cross-instance window discovery, capture, and naming

**Date:** 2026-08-24
**Status:** Phase 1 (Fixes 1-4, §2-5) implemented on branch `feat/agent-app-api-window-discovery`. Phase 2 (Fix 5, cross-instance naming, §6) and Fix 6 (§7, orphan discovery) remain draft/not started. §8's open item (Shell-tool exit-201 mystery) remains unresolved — not attempted here, per its own note.
**Trigger:** `docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md` — a live session where an agent needed to find, screenshot, and rename a `task dev` build's window while working, and hit real blockers at every step: an ambiguous `CaptureWindow` match silently returned an unrelated (and in this case sensitive) window instead of the intended one; no tool exists to enumerate candidate windows before capturing; a legitimately-running window screenshotted as unexplained solid black; `SetName` cannot reach a window in a different `agentmux-srv` instance (confirmed structural); and a raw external Win32 rename used as a workaround got silently overwritten by AgentMux's own reactive title system, in a way that looked like a tool bug but wasn't.

**Scope:** `agentmux-mcp/src/main.rs` (tool definitions + `capture_window_impl`), `agentmux-srv/src/server/mod.rs` (`handle_window_name` and a new cross-instance name-request path), plus a new lightweight local-instance discovery mechanism. No changes to the UI-automation click/input boundary (`SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`'s screenshot-only design stays screenshot-only — this spec is about *finding and naming* windows more reliably, not adding input injection).

---

## 1. Why

`CaptureWindow` is, by design, "the sole deliberately narrow exception" that reaches outside an agent's own `agentmux-srv` instance (`REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md`). That's the right scope boundary — this spec doesn't propose widening it to input injection or arbitrary cross-instance control. But within that narrow screenshot-only scope, the tool surface has real, fixable gaps: it can't be queried cheaply before spending a capture call, its own documented ambiguity-handling doesn't match its implementation, it gives no signal on a black/blank result, and there is no companion naming capability at all — forcing agents toward raw OS-level workarounds that don't actually work reliably against this specific app (because AgentMux's own frontend actively, reactively manages its window title and will silently overwrite an externally-forced rename).

## 2. Fix 1 — `DiscoverWindows`: a read-only, no-image-cost enumeration tool

New MCP tool. Same underlying enumeration `capture_window_impl` already does (`xcap::Window::all()` filtered by `app_name()` prefix `"agentmux"`), reused as a shared function, but returning structured data instead of a screenshot:

```
DiscoverWindows(include_self: bool = false) -> {
  windows: [
    { pid: u32, title: string, exe_path: string, is_self: bool }
  ]
}
```

- `exe_path` (already available from `xcap`'s underlying process query, same source `own_instance_pids()` already reads) lets a caller distinguish *which* AgentMux build/instance a window belongs to — e.g. a `task dev` build under `dist/cef-dev` vs. an installed portable build — without needing to guess from the title, which per §4/§5 below is not a stable signal for this app.
- `is_self` reuses the existing `own_instance_pids()` exclusion logic, but as a flag rather than a hard filter, so a caller can still see (but clearly identify) their own window if they want to.
- No image is written, nothing touches `capture_window_dir()`, no audit-log entry (nothing sensitive is disclosed beyond title/pid/path, which `CaptureWindow`'s own audit log already treats as safe to log).
- Directly replaces the blind `index: 0, 1, 2, ...` guessing from blocker #1/#2 in the report — a caller runs this once, sees the real candidate list, then targets precisely.

## 3. Fix 2 — `CaptureWindow` accepts a stable `pid` target, not just a racy title substring

Add an optional `pid` parameter as an alternative to `title_contains`/`index`:

```
CaptureWindow(pid: u32) -> (same result as today)
CaptureWindow(title_contains: string, index?: number) -> (existing behavior, kept for convenience)
```

When `pid` is given, skip title matching entirely — find the window(s) owned by that PID directly (same `GetWindowThreadProcessId`-equivalent xcap already does internally for the `app_name()` filter) and capture the (first) visible one. This is the single highest-value fix from the report: per §5's finding, an AgentMux window's *title* is under continuous reactive control by the app itself and is not a stable identifier to search by at all — a PID (obtained from `DiscoverWindows`, or already known because the caller launched the process themselves, as in the report's own scenario) doesn't have that problem for the lifetime of the process. `title_contains` remains supported for the case where a caller doesn't have a PID handy, but its docs should be updated to note it's inherently racy against this app's own title-management behavior, not just "case-insensitive substring, may be ambiguous."

## 4. Fix 3 — ambiguous title match returns the real candidate list, matching the tool's own existing docstring

Bug fix, not a new capability: `CAPTURE_WINDOW_TOOL`'s docstring already promises *"The error message lists all matching AgentMux window titles when this is ambiguous"* — `capture_window_impl` doesn't do this; it silently captures `index` (default 0) with no signal that other matches existed. Fix: when `title_contains` (without an explicit `pid`) matches more than one window and `index` was not explicitly provided, return an error listing every match (`title`, `pid`) instead of silently picking one. This alone would have prevented blocker #1 in the report — the accidental capture of an unrelated, sensitive session was possible specifically because the tool silently proceeded instead of surfacing the ambiguity it claims to detect.

## 5. Fix 4 — blank/black-frame detection with a short bounded retry

`capture_window_impl` calls `window.capture_image()` once, with no sanity check on the result. Add: after capture, cheaply check whether the image is a single solid color (or below a low variance/entropy threshold — a cheap check, not real image analysis) — if so, and the target process was created less than some short threshold ago (a few seconds; process start time is already obtainable via the same process-info call `own_instance_pids()` uses), retry the capture up to 2-3 times with a short delay (e.g. 300-500ms apart) before giving up. Whether or not the retry resolves it, include a `likely_unrendered: bool` flag in the tool result so a caller isn't left guessing (as in blocker #3) whether a black image means "try again shortly" or "this is really what the window shows."

## 6. Fix 5 — the real fix for cross-instance naming: a local instance-discovery broker, not a hardened workaround

This is the structurally bigger piece, and worth being explicit about the trade-off: **do not** try to make raw external `SetWindowText` reliable against this app. Per the report's §5 finding, AgentMux's frontend will keep winning that race by design (its title is reactively recomputed on tab switch / workspace rename / any window opening or closing anywhere on the machine) — hardening around that behavior means fighting the app's own architecture indefinitely. The actual fix is to let a rename request go through the *same* reactive pipeline the app already uses for its own instance (`window:displayname` meta → `document.title` effect → `SetWindowTextW`), just triggered from a different instance than the one the window belongs to.

That requires solving what today's tools structurally can't: an agent in instance A doesn't know instance B's `AGENTMUX_LOCAL_URL`/auth key (each MCP process reads its own once at startup and never repoints — confirmed in the report). Proposed shape:

1. Each `agentmux-srv` instance, on startup, registers a small local record — PID, `AGENTMUX_LOCAL_URL` (effectively just its bound port, since it's loopback-only), and a short-lived instance token — in a well-known local location (e.g. a file under a shared `AGENTMUX_DATA_HOME`-adjacent directory, keyed by PID, cleaned up on clean shutdown and treated as stale/ignorable if the PID no longer exists on a read — same liveness-check discipline `SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md` had to retrofit onto the *unrelated* internal HWND cache after it shipped without one; worth building this broker with that lesson already applied rather than needing a follow-up fix).
2. A new MCP tool, `SetForeignWindowName(pid: u32, name: string)`: resolves `pid` → owning `agentmux-srv` instance via that registry (an owning instance's PID is discoverable the same way `DiscoverWindows`/`CaptureWindow` already resolve `app_name()`-filtered windows to their owning process — the new piece is only the *registry lookup* from "which srv instance owns this browser-process PID," not the window-to-process resolution itself, which already exists), then makes the same `WindowNameRequest` POST `SetName` already makes today, just addressed at instance B's registered local URL/token instead of the caller's own.
3. Server-side, `handle_window_name` needs no logic change — it already just resolves `window_id` and writes `window:displayname` meta on whichever instance receives the request; the only new requirement is accepting the request from a token issued by the discovery-registry step above rather than only from same-instance callers, scoped narrowly (loopback-only, short-lived token, rename-only — not a general cross-instance RPC channel).

This is a real, multi-piece addition — flagging it as Phase 2, separate from Fixes 1-4, which need no new cross-instance channel at all and are independently shippable.

## 7. Fix 6 — orphan discovery for `Shell`-tool processes independent of a held `shell_id`

`ShellStop`'s `kill_tree` (PID/process-group scoped, `shell_node.rs:176-186`) is already the right primitive — the gap is discovery when the `shell_id` that would let you call it has been lost (session interruption, or a process that outlived the calling agent's own turn). Proposal: tag every `ShellNodeRunner`-spawned process with an identifiable marker at launch — an environment variable (e.g. `AGENTMUX_SHELL_TOOL_SESSION=<agent-id>:<shell_id>`) inherited by the whole process tree — and add a `ListOrphanedShells()` tool that enumerates currently-running processes carrying that marker whose `shell_id` is no longer present in the live `ShellSessionRegistry` (i.e., the registry entry expired/was evicted per `MAX_EXITED_STATUS`, or the srv instance that spawned it was restarted, but the child process itself survived). This directly replaces the manual `Get-CimInstance Win32_Process` + command-line pattern-matching archaeology from blocker #7 with a single, scoped, purpose-built call.

## 8. Open item — NOT proposed here, needs its own investigation first

Blocker #6 in the report (`mcp__agentmux__Shell`-launched `npm run dev` dying at exit code 201, reproducibly, at the same point, with no lifetime-cap code found anywhere in `ShellNodeRunner` to explain it) has no confirmed root cause. Recommend: add exit-detail logging to `ShellNodeRunner` (capture and surface the child's actual termination reason — exit code *and*, on Windows, whether it was `TerminateProcess`-style vs a clean exit — not just the wrapper's own generic code) so a future report can pin this down with real data instead of speculation. Do not attempt a fix in this spec without that diagnostic step first — guessing at a cause here (environment variable difference, stdio handling, working-directory resolution) would be exactly the kind of blind iteration this whole report is about avoiding.

## 9. Files touched (Fixes 1-4, Phase 1)

```
agentmux-mcp/src/main.rs                    MODIFY — add DISCOVER_WINDOWS_TOOL + handler (reuses
                                              capture_window_impl's enumeration as a shared fn);
                                              add `pid` param to CAPTURE_WINDOW_TOOL; fix ambiguous-
                                              match handling to return the candidate list; add
                                              blank-frame retry + `likely_unrendered` flag to
                                              capture_window_impl.
```

## 10. Files touched (Fix 5, Phase 2 — separate follow-up)

```
agentmux-srv/src/main.rs                    MODIFY — register/deregister this instance's local
                                              discovery record on startup/clean shutdown.
agentmux-srv/src/server/mod.rs              MODIFY — accept SetName-equivalent requests bearing a
                                              valid cross-instance discovery token, in addition to
                                              same-instance callers.
agentmux-mcp/src/main.rs                    MODIFY — add SetForeignWindowName tool (discovery
                                              lookup + cross-instance POST).
(new) local instance-discovery registry     NEW — file-based, PID-keyed, liveness-checked on read.
```

## 11. Files touched (Fix 6)

```
agentmux-srv/src/backend/shell_node.rs      MODIFY — tag spawned process trees with an
                                              AGENTMUX_SHELL_TOOL_SESSION env var; add orphan-
                                              enumeration support (cross-reference tagged live
                                              processes against the current ShellSessionRegistry).
agentmux-mcp/src/main.rs                    MODIFY — add ListOrphanedShells tool.
```

## 11a. Implementation notes (Phase 1, as actually shipped)

- `enumerate_agentmux_windows()` is the single shared enumeration function `DiscoverWindows` and `capture_window_impl` both call — replaces the old inline `xcap::Window::all()` + filter logic that used to live only inside `capture_window_impl`. Reuses `own_instance_pids()` unchanged.
- Fix 4's retry gate is simpler than originally proposed: the spec draft suggested only retrying when the target process was recently created (needing process start-time plumbing). Shipped version drops that gate — `looks_unrendered()` triggers up to 2 retries (400ms apart, ~800ms worst case) whenever a captured frame is near-uniform color, regardless of process age. Simpler, and the worst case (a genuinely solid-color-themed window burns an extra ~800ms confirming it's really solid) was judged an acceptable trade-off against needing new process-age plumbing for a heuristic that's already approximate.
- `looks_unrendered()` samples ~200 evenly-spaced pixels with an 8-per-channel tolerance — cheap and approximate by design (a real solid-color-themed window will also trip it; that's fine, it only gates a short retry and a hint flag, never a hard failure).
- The pre-existing `title_contains` required-field validation moved from JSON-schema `"required"` to runtime: since `pid` is now a valid alternative target, the schema no longer marks either field as unconditionally required — `capture_window_impl` returns a clear error if neither is given.
- `audit_log_capture_window`'s NDJSON field renamed `title_contains` → `query` (now `"pid=N"` or `"title_contains=\"...\""`, whichever targeting mode was used) — the one intentional behavior change to the audit log's shape, needed so a pid-targeted call logs something meaningful instead of an empty title_contains.
- Added 3 new unit tests for `looks_unrendered()` (solid-color flagged, clearly-varied-region not flagged, within-tolerance noise still flagged) and updated 2 existing tests (`audit_log_capture_window_appends_ndjson_for_success_and_failure`'s field-name assertion; `all_tool_defs_are_valid_json_with_names`'s tool-count pin, 35→36).
- Verified: `cargo build -p agentmux-mcp` clean, `cargo test -p agentmux-mcp` 16/16 passing, `cargo clippy -p agentmux-mcp --all-targets` zero new warnings (2 pre-existing warnings at lines ~3256/3258 are in unrelated code, confirmed unchanged by this diff). `cargo fmt --check` was NOT applied — the file already has 62 pre-existing formatting deviations from a clean rustfmt run, confirmed present in the untouched `origin/main` version before this change, so fmt isn't treated as enforced for this file; running it would have bundled an unrelated 62-hunk reformat into this diff.

## 12. Acceptance criteria

1. `DiscoverWindows` returns title/pid/exe_path/is_self for every AgentMux-owned top-level window on the machine, with no image written and no audit-log entry.
2. `CaptureWindow(pid: N)` captures the window owned by PID `N` directly, with no title matching involved.
3. `CaptureWindow(title_contains: "X")` where `X` matches 2+ windows and no `index` was given returns an error listing every match's title+pid, rather than silently capturing index 0. (Regression test: the exact scenario from blocker #1 — a generic substring matching an unrelated window plus the intended one — must surface both, not silently return one.)
4. A capture of a solid/near-solid-color frame from a process created within the retry threshold triggers at least one retry before returning; the result always includes `likely_unrendered`.
5. (Phase 2) `SetForeignWindowName` successfully renames a window belonging to a different, currently-running `agentmux-srv` instance, and the rename survives that instance's own reactive title recomputation (i.e., it went through `window:displayname`, not a raw OS call) — verified by triggering a tab switch on the target instance after the rename and confirming the title is unchanged.
6. `ListOrphanedShells` finds a `Shell`-tool-spawned process tree whose owning agent session and `shell_id` have both been lost (simulated by killing the calling MCP process without calling `ShellStop` first), and the returned identifier is sufficient to `taskkill`/`kill_tree` it without manual PID cross-referencing.
7. No change to the UI-automation input-injection boundary — `CaptureWindow` and its Phase-1 extensions remain screenshot-only, per `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`'s explicit scope decision.
