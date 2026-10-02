---
name: run
description: Launch and verify AgentMux's own `task dev` build on Windows — the MAX_PATH subst workaround, a real free-port check instead of trusting the hash, how to confirm the window is actually ready (poll state, not logs), and a safe cleanup pattern for a host running several agents' dev sessions at once.
---

# Running AgentMux itself (`task dev`, Windows)

This is the verified path for launching AgentMux's own dev build to check a
change, superseding a generic Electron-app procedure. It exists because an
agent spent about an hour rediscovering all of this from scratch — see
`docs/retro/RETRO_AGENT_TASK_DEV_LIVENESS_AND_PORT_COLLISION_2026_09_27.md`
for the full incident and root-causing. Follow this verbatim; don't
re-diagnose the same four problems.

**Scope note:** this is dev-workflow friction, not an isolation bug. Nothing
below is needed because AgentMux's multi-instance guarantees (I1–I6 in
`docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`) are
broken — they hold. This is about an *agent* reliably launching and
*observing* its own instance on a host where several peers are doing the
same, which those invariants don't cover.

## 1. Check your workspace path length first

Windows' 260-character `MAX_PATH` breaks the vendored CEF C++ build
(`libcef_dll_wrapper`) with a misleading error:

```
fatal error C1083: Cannot open compiler generated file: '': Invalid argument
```

Agent workspace paths (`~/.agentmux/agents/<agent-id>/work/.../agentmuxai__agentmux`)
are routinely long enough to hit this. Check before building, not after a
confusing failure:

```bash
pwd -P | wc -c   # if this is over ~100, assume trouble; CEF's own nested
                  # test/ tree + generated filenames eat the rest of the budget
```

If it's long, map a short drive letter **before** building — this is a
verified fix (`docs/retro/RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md`),
not a fallback to try after failing:

```bash
cmd //c "subst K: <repo-path-with-backslashes>"   # one whole double-quoted
# string, no nested quotes, no bare unquoted backslashes — see "Gotchas" below
cd /k
```

Undo when done: `cmd //c "subst K: /D"` (run from a different directory —
you can't remove a mapping you're currently inside).

## 2. The port: let the Taskfile pick it; if you must override, use `vite-port.sh`

`Taskfile.yml`'s `dev` task derives `AGENTMUX_VITE_PORT` from a 200-slot hash
of your workspace path (5173–5372). With several agents' clones on one host this
can collide, and the task's own collision-recovery has two gaps:

- Its "is this port busy" pre-check is an HTTP `curl`, which can't see a
  process that has the socket bound but is hung/not answering HTTP.
- Its "is this my own orphaned Vite" ownership check compares raw path
  strings — it stops working for the same clone the moment you apply the
  `subst` workaround above, because the path string changes.

**Default: set nothing.** If `task dev` reports the port is held by another
agent's Vite, override it with the script, which checks both things that matter:

```bash
AGENTMUX_VITE_PORT=$(bash scripts/vite-port.sh pick) task dev TITLE="<your-agent-name>"
```

`pick` returns the first port from 5300 up that nothing is listening on **and
that Chromium will load pages from**. The second condition is not optional: the
dev window is a Chromium window, and Chromium refuses certain ports outright
(`ERR_UNSAFE_PORT`) — 6000 (X11), 6665–6669 (IRC), 5060/5061 (SIP), 4045, 2049
and a few others. A dev instance pointed at one of them starts, Vite answers
200, and the window then flickers forever with no message. This cost an agent a
long debugging session on 2026-10-02 (port 6000 — see
`docs/specs/SPEC_DEV_VITE_UNSAFE_PORT_GUARD_2026_10_02.md`). `task dev` now
refuses such a port up front, but don't walk into it.

**Do not write your own free-port probe.** The earlier recipe here used bash's
`/dev/tcp` against `127.0.0.1`. On Windows Vite binds `localhost` as `[::1]`
only, so that IPv4-only probe reported 5999 free while another agent's Vite held
it, and the walk upward then landed on the blocked 6000. (#4205 added an IPv6
probe to the recipe; `vite-port.sh` makes the whole recipe unnecessary.) It uses
`netstat -ano` on Windows, which lists listeners of both families, and `lsof`
(or `ss`) elsewhere.

If you still see `Error: Port <n> is already in use` / repeated
`vite.config.ts changed, restarting server...`, that port is genuinely stuck
(usually your own earlier orphan) — go to §4, don't just retry the same port.

## 3. How to know it's actually ready — poll state, don't tail logs

**Don't tail the log file to see if it's working.** Redirected `task dev`
output showed essentially nothing until the process exited, for reasons not
conclusively isolated (see the retro — this wasn't simple, fully-explained
CRT buffering; something in the Go/shell/Rust/Node chain suppresses almost
all incremental output, mechanism unresolved). For a long-running successful
session that never exits on its own, a log file may show **nothing, ever**,
even though everything is fine.

Poll OS-visible state instead:

```bash
powershell -NoProfile -Command "Get-Process | Where-Object {\$_.MainWindowTitle -like '*<your-agent-name>*'} | Select-Object Id,ProcessName,MainWindowTitle"
```

**Match by substring, not exact equality.** `TITLE=` becomes the window's
*display name*, not the whole OS title — the real title is built by
`frontend/util/window-title.ts`'s `formatWindowTitle()` as
`"<displayName> - <tabName> - AgentMux"` (or `"<displayName> - AgentMux"` with
no tab). You'll see things like `"Lark - Tab 1 - AgentMux"` from other
agents' own sessions on a shared host — that's normal, not a collision.

**Expect a real delay between "window exists" and "title is set."** The
title is applied by `frontend/app-init.ts` only after `initMuxWrap()`
finishes — i.e., after the frontend has actually loaded and connected. A
window that's `Responding: True` with a bare `"AgentMux"` title for a while
is not stuck; it just hasn't finished init yet. If it *never* progresses past
plain `"AgentMux"` after a minute or two, the frontend is probably stuck
behind a Vite problem (§2), not slow — check
`Get-NetTCPConnection -LocalPort $port` to confirm something is actually
listening.

## 4. Cleanup: scope every kill to a confirmed-yours PID

On a host running several agents, a loose filter like `*cef-dev*` or `*K:*`
can match other agents' live processes, not just your own strays. **Always
confirm ownership before killing anything:**

```bash
powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter 'ProcessId=<pid>').CommandLine"
# only proceed if the printed CommandLine actually references YOUR workspace path
```

Only then:

```bash
powershell -NoProfile -Command "Stop-Process -Id <pid> -Force -ErrorAction SilentlyContinue"
```

A CEF app's process tree needs two passes sometimes (parent exits, orphaned
children take a moment to follow) — re-check and re-run once if any of your
own PIDs remain.

## 5. Backgrounding: use `run_in_background`, don't try to fully detach

A `run_in_background: true` Bash tool call survives many subsequent,
unrelated tool calls in the same session without incident — that's the
working pattern. Prefer it over trying to detach the process yourself.

If you do reach for PowerShell's `Start-Process` (e.g. to launch outside
this session's own process tree entirely), **root cause found**: on Windows
PowerShell 5.1, `-ArgumentList` given as a string **array** — e.g.
`-ArgumentList @('-lc', 'echo hi; pwd')` — gets flattened into the child's
command line *without quoting each element*. `bash` then receives
`-lc echo hi; pwd` as three separate argv entries instead of `-lc` plus one
command string; `bash -c`'s multi-word script collapses to just its first
word (`echo`), and everything after becomes an ignored positional parameter
(`$0`, `$1`, ...). The process still starts and exits 0 — it silently did
nothing useful, which is exactly the symptom that made this hard to spot
(no error, no crash, just no effect).

**Fix: pass `-ArgumentList` as a single pre-quoted string, not an array:**

```powershell
$cmd = 'cd /some/path && echo hi && pwd'
Start-Process -FilePath 'C:\Program Files\Git\bin\bash.exe' `
  -ArgumentList "-lc `"$cmd`"" -RedirectStandardOutput $log -WindowStyle Hidden
```

Verified: the array form above produces an empty log and exit code 0 with
nothing executed; the single-string form runs the full multi-word command
and captures its real output. (`cmd.exe` targets were not affected by this
particular bug — its own `/c` argument doesn't get split the same way — so
if your only prior test was a plain `cmd.exe` command, that isn't evidence
against this fix mattering for `bash.exe`.)

If you kill a background `task dev` job yourself later (e.g. via
`taskkill`), that's you, not the environment — don't mistake it for an
external interrupt.

## Gotchas

- **`cmd //c` argument quoting on Windows paths.** Pass the *entire* command
  as one double-quoted string (`cmd //c "subst K: C:\Users\..."`), not
  separate unquoted words — unquoted backslashes get consumed as Bash escape
  characters and silently strip out of the path. Don't mix single- and
  double-quotes for the same call; that produced a spurious leading backslash
  in testing (`subst` then reports `Path not found`).
- **`task dev`'s window starts titled plain `"AgentMux"`** even for a
  perfectly healthy launch, before init finishes (§3) — don't read that as a
  problem on its own.
- **Other agents' sessions on the same host are normal, not a bug.** Seeing
  other `Lark`/`Korp`/etc.-titled windows, or other `node.exe`/`cargo.exe`
  processes with high memory, is expected on a shared host — don't clean
  those up.

## References

- `docs/retro/RETRO_AGENT_TASK_DEV_LIVENESS_AND_PORT_COLLISION_2026_09_27.md` — full incident, evidence, and the plan this skill implements (P1).
- `docs/specs/SPEC_DEV_VITE_UNSAFE_PORT_GUARD_2026_10_02.md` — why §2 forbids a hand-rolled probe and a blocked port.
- `docs/retro/RETRO_CEF_C1083_PARALLEL_BUILD_RACE_2026_07_14.md` — the `MAX_PATH` root cause.
- `docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md` — the `AGENTMUX_VITE_PORT` mechanism this skill works around.
- `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` — the I1–I6 invariants this skill's guidance is careful not to violate.
- `frontend/util/window-title.ts`, `frontend/app-init.ts` — the title-formatting and application logic §3 relies on.
