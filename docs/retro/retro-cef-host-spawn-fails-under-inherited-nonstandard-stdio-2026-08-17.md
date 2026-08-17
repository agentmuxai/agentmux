# Retro: `agentmux.exe` exits instantly (no window, no log, no data dir) when launched from a deeply-nested agent shell

**Date:** 2026-08-17
**Trigger:** Building a fresh isolated portable (`task package`) to verify a
backend change and launching it from an AgentMux agent's own Bash tool
(itself running inside another live AgentMux instance, several process
generations deep through `agentmux-bashwrap.exe`/MSYS bash). The launcher
exited in well under a second, exit code 1, zero stdout/stderr, and never
created its per-build-channel data directory at all — no window, no log
line, nothing to debug from.
**Status:** Root-caused and reproducibly confirmed/fixed by launch method
(not yet fixed in source — filing for follow-up).

---

## 1. Symptom

`./agentmux.exe` (portable, freshly built, confirmed to include the target
commit) ran and returned exit code 1 in under a second when launched from a
Bash tool call:

```
$ ./agentmux.exe
exit=1
```

No stdout, no stderr, no crash dump under `C:\CrashDumps\`, and — most
confusingly — **no per-build-channel directory was ever created** under
`~/.agentmux/channels/<channel>/`, meaning the failure happened before the
launcher got anywhere near its normal data-dir bootstrap. `timeout 5
./agentmux.exe` confirmed it wasn't hanging — it genuinely exits in well
under a second.

## 2. What it wasn't (ruled out, with evidence)

Before finding the real cause, these were checked and ruled out:

1. **Job Object UI restrictions.** `agentmux-launcher/src/job_object.rs`'s
   `create_job_object()` only sets `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` —
   no `JOBOBJECT_BASIC_UI_RESTRICTIONS`, no desktop/handle limits, nothing
   that would block a nested child from creating windows.
2. **Desktop/window-station access in general.** A plain
   `System.Diagnostics.Process.Start()` launch of `notepad.exe` from the
   exact same shell context succeeded immediately, proving the process
   tree has normal interactive-desktop access.
3. **Process token / integrity level.** `whoami /groups` showed `Mandatory
   Label\Medium Mandatory Level` — a completely normal, unrestricted
   token. No AppContainer, no Low integrity, no sandbox.
4. **CEF host PE subsystem.** Confirmed via the PE optional-header
   `Subsystem` field: `2` (GUI/WINDOWS), matching
   `agentmux-cef/src/main.rs`'s `#![windows_subsystem = "windows"]`. Ruled
   out a console-auto-allocation conflict (that mechanism only applies to
   CUI/subsystem-3 targets).
5. **`CREATE_SUSPENDED` itself.** Not universally broken in this
   environment — the *same failing launch attempt*'s own log showed the
   srv backend (also spawned with `CREATE_SUSPENDED`, by
   `srv_spawner.rs`) starting up completely normally and reaching
   `ready:` in its log. Whatever's wrong is specific to the CEF **host**
   spawn, not the flag or the environment in general.

## 3. Root cause

The real launcher log (`~/.agentmux/logs/agentmux-launcher.log`, shared
across all instances/channels — the per-channel dir never existed to check
instead) had the actual answer the whole time, once grepped directly
instead of relying on the empty captured stdout:

```
srv 62176 ready: ws=127.0.0.1:56293 web=127.0.0.1:56292 instance=v0.55.11 pending_migrations=0
failed to spawn CEF host: The request is not supported. (os error 50)
FATAL: could not start CEF host — terminating
```

`agentmux-launcher/src/host_spawn.rs::spawn_host_supervised` builds its
`tokio::process::Command` like this (lines ~39–51):

```rust
let mut host_cmd = tokio::process::Command::new(real_exe);
host_cmd
    .args(args)
    .env(...)
    .creation_flags(CREATE_SUSPENDED)
    .kill_on_drop(false);
// no .stdin()/.stdout()/.stderr() calls — inherits the parent's stdio
```

Compare `agentmux-launcher/src/srv_spawner.rs`'s spawn of the backend
(lines ~306–336), which **works**, in the same failing launch attempt:

```rust
let mut cmd = Command::new(&backend_path);
cmd.args([...])
    .env(...)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(false);
cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
```

**The difference is stdio.** `srv_spawner` gives its child fresh,
tokio-created anonymous pipes for all three streams. `host_spawn` inherits
whatever stdio handles its own parent process happened to have — normally
harmless (a user double-clicking `agentmux.exe` from Explorer inherits
Explorer's unremarkable default handles), but in this case the launcher's
own stdio traced back through several generations of MSYS/Git-Bash
processes (`agentmux-bashwrap.exe` → `bash.exe` → `bash.exe` → …) whose
file descriptors are MSYS-emulated, not plain Win32 file/pipe handles.

**Confirmed by direct experiment, not just inference:**

- Launching with those inherited MSYS-lineage handles (via `cmd.exe /c`,
  via bare `run_in_background`, via `&`-backgrounding — every variant
  tried) → instant exit, `os error 50`, exactly reproducing the failure.
- Launching the identical binary via
  `Start-Process -RedirectStandardOutput <file> -RedirectStandardError <file>`
  (PowerShell, which gives the child genuine Win32 file handles instead of
  inherited MSYS ones) → **the app started cleanly**: frontend painted,
  main window created (`EVENT_OBJECT_CREATE ... class=Chrome_WidgetWin_1`),
  full normal startup-bench log, still running after 4+ seconds.

So: `CREATE_SUSPENDED` + inherited stdio handles that originate from an
MSYS pseudo-console/pipe chain is the trigger. `CreateProcess` appears to
reject this specific combination for a GUI-subsystem target with
`ERROR_NOT_SUPPORTED` — `srv_spawner`'s console-subsystem target never hits
it because it never inherits those handles in the first place (it pipes
its own).

## 4. Fix direction (not yet implemented)

Make `host_spawn.rs` explicitly configure stdio instead of relying on
inheritance — mirroring `srv_spawner.rs`'s already-working pattern. The CEF
host writes its own log file (`agentmux_cef::bootstrap`'s tracing setup) and
doesn't need a console at all (`windows_subsystem = "windows"` already
means it has none), so `Stdio::null()` for all three streams is likely
sufficient and simplest; `Stdio::piped()` (matching srv exactly) is the
more conservative match if anything downstream ever wants to capture host
output. Either change should be tested specifically by launching from a
Bash-tool/MSYS-nested shell (the case that reproduces it), not just from
Explorer (the case that's always worked and would mask a regression here).

## 5. Why this was never noticed before

Every normal launch path — double-clicking the exe, a desktop shortcut,
`explorer.exe`-initiated anything, even `task dev`'s own launcher
invocation from a plain Windows shell — starts with unremarkable, real
Win32 stdio handles (often none at all, or a real console). It takes a
deeply MSYS/bash-nested ancestry specifically to produce handles that trip
this. That's an increasingly common case for this project though: any
agent verifying its own backend changes by building and launching a fresh
portable from inside its own Bash tool call — exactly what this session
was doing — hits it every time.

## 6. Sources

- `agentmux-launcher/src/host_spawn.rs:14-70` (`spawn_host_supervised`,
  the failing call site)
- `agentmux-launcher/src/srv_spawner.rs:300-374` (the working comparison)
- `agentmux-launcher/src/job_object.rs` (ruled out: no UI restriction
  flags)
- `agentmux-cef/src/main.rs:8` (`windows_subsystem = "windows"`)
- `~/.agentmux/logs/agentmux-launcher.log` (live evidence: `v0.55.11 srv
  62176 ready`, `failed to spawn CEF host: The request is not supported.
  (os error 50)`, `FATAL: could not start CEF host — terminating`)
- Live experiment: `Start-Process -RedirectStandardOutput/-RedirectStandardError`
  launch succeeded where every MSYS-stdio-inherited variant failed
  identically (channel `local-main-b28b7a-0e5d07ad`, confirmed via its
  `agentmux-host` log: full startup-bench trace, real `Chrome_WidgetWin_1`
  window creation)
- `docs/retro/retro-md-drop-window-hijack-and-55-6-relaunch-failure-2026-08-16.md`
  — a different launcher-silent-exit failure mode (schema-version
  mismatch + no fatal dialog on that specific path) that looked similar at
  first glance but is unrelated; ruled out early since srv opened its
  database fine here (`pending_migrations=0`, reached `ready`).
