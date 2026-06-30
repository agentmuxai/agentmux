# Graceful OOM Exit — Own the Death, Explain the Reason

**Date:** 2026-06-29
**Status:** Draft — design + phased plan
**Affected:** AgentMux on Windows (CEF/Chromium 148). Extends the memory-pressure corpus.

> **Builds on:** `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`, `SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md`,
> `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`. Read those first — this assumes the shipped
> gated recovery, `mem_supervisor.rs`, and commit gauge.

---

## 1. The problem (observed)

User returned to a Win10 box that had been under commit pressure overnight. Traktor (a memory hog)
was fine. AgentMux was **unrecoverable**:
1. The **Reload/recovery screen was half-rendered** — broken UI.
2. Windows showed a **raw OS error dialog with a memory address** (an access-violation / fault box).
3. No clear explanation of *why* AgentMux died or whether anything was saved.

We can't make Chromium tolerate OOM — it is configured to **crash rather than return null** from any
allocation (`base::internal::OnNoMemoryInternal` on the stack; exception `0xE0000008`). See §7. So the
goal is not "don't die" — it's **die gracefully and legibly**: clean up durable state, suppress the
ugly OS fault box, and show one clear native message stating the reason.

## 2. Why each symptom happens (root cause, mapped to code)

| Symptom | Root cause | Code |
|---------|-----------|------|
| **Half-broken Reload screen** | The recovery / low-memory pause page is **HTML rendered by a freshly-spawned CEF renderer** (`frame.load_url("data:text/html;base64,…")`). Under true commit exhaustion that new renderer *also* can't get memory, so it paints partially or crash-loops. The recovery UI depends on the exact resource that's exhausted. | `agentmux-cef/src/client/mod.rs` `on_render_process_terminated` (~1547), `memory_paused_page` (~2211), `crash_loop_terminal_page` |
| **Raw Windows fault box w/ memory address** | The **host and CEF subprocesses don't suppress the GPF error box**. Only the launcher sets `SEM_FAILCRITICALERRORS`; nothing sets `SEM_NOGPFAULTERRORBOX`, and there's **no `SetUnhandledExceptionFilter`**. So an unhandled `0xE0000008` / `0xC0000005` falls through to Windows' default crash dialog. | `agentmux-cef/src/lib.rs` init (no SEH filter); launcher `main.rs:~72` (`SEM_FAILCRITICALERRORS` only) |
| **No reason / unclear if saved** | The one good native dialog (`show_fatal_dialog` → `MessageBoxW`, with reassuring text) only fires in the launcher **after a 5-minute relaunch-deadline expires** — not at the moment of death, and not when the host hard-crashes vs. is supervised. | `agentmux-launcher/src/main.rs` `show_fatal_dialog` (~2155), `OOM_GIVEUP_BODY` |

**Key architectural asset:** the **launcher is a tiny, separate process that survives OOM** (the
srv survived the 6/26 kill too). It has memory headroom when the CEF fleet does not. The launcher —
not the dying host — should own the "explain + verify cleanup" step.

## 3. Design — four pillars

### Pillar A — Suppress the ugly OS fault box; install our own last-resort handler
On **every** AgentMux process (host + each CEF subprocess + launcher), at the earliest entry point:
- `SetErrorMode(prev | SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS)` — augment, never overwrite
  (the classic bug: overwriting clobbers other flags — read-modify-write). Also consider
  `WerSetFlags(WER_FAULT_REPORTING_NO_UI)` to keep WER silent.
- `SetUnhandledExceptionFilter(amx_last_breath)` — a minimal SEH filter that runs on `0xE0000008`
  (Chromium OOM) and `0xC0000005` (AV). It must do the **least possible work** (see Pillar B) and
  then signal the launcher, returning `EXCEPTION_EXECUTE_HANDLER` so Windows shows nothing.

### Pillar B — A pre-allocated "parachute" so cleanup can run when the heap is exhausted
At startup, reserve a small **committed** block (e.g. 8–16 MB) in the host and launcher. The
`amx_last_breath` filter's **first action is to free the parachute**, giving the handler enough heap
to: write a one-line crash-reason file, flush nothing heavy, and post a message to the launcher over
the existing host-pipe (or a named event + a tiny shared-memory reason struct that needs no
allocation). This is the standard "release reserve to die gracefully" pattern — without it, the
handler itself OOMs and you're back to the raw fault box.

### Pillar C — Native, renderer-independent fatal UI (don't ask Chromium to paint your apology)
Introduce a **commit floor below which we never attempt an HTML recovery page** (call it
`PAINT_FLOOR_MB`, below the existing `RESUME_FLOOR_MB`). Below `PAINT_FLOOR_MB`:
- Do **not** call `frame.load_url(data:…)` — a fresh renderer can't paint.
- Instead the **launcher** shows a native **Task Dialog / `MessageBoxW`** (no CEF, no renderer, no
  GPU) explaining the situation, with a single **Reopen** action.

Decision ladder on renderer death:
```
renderer OOM (PROCESS_OOM)
 ├─ commit_free ≥ RESUME_FLOOR_MB ........ HTML recovery page (current behavior — fine, memory exists)
 ├─ PAINT_FLOOR_MB ≤ free < RESUME_FLOOR . HTML pause page, one attempt; if it re-OOMs once → escalate
 └─ free < PAINT_FLOOR_MB ............... SKIP html; signal launcher → native dialog + clean teardown
```

### Pillar D — Guaranteed durable cleanup on abnormal exit (not just clean exit)
Today WAL checkpoint + saga-cancel run only on **clean** shutdown. On an OOM abort they're skipped
(recovered lazily next launch). Make the **launcher** — which observes host death and has headroom —
run the durable closeout when it classifies the exit as `SystemOom`/`Abnormal`:
- `PRAGMA wal_checkpoint(TRUNCATE)` on `objects.db` / `filestore.db` (srv may already be dead — the
  launcher can open them read-write briefly, or signal a surviving srv to do it).
- `cancel_all_in_flight` saga closeout (already exists — ensure it runs on the abnormal branch).
- Job-object `KILL_ON_JOB_CLOSE` already tree-kills orphans — keep, but sequence it **after** the
  closeout + dialog so nothing is reaped before state is flushed.

## 4. The message the user should see

Native dialog (launcher), shown **at the moment of unrecoverable exit** (not after a 5-min wait):

> **AgentMux had to close — out of memory**
> Windows ran out of memory, so AgentMux couldn't keep running. This isn't a crash in your work —
> your panes, agents, and sign-ins are saved.
>
> At the time, the most memory was used by: *Traktor (0.9 GB), 3 agents (1.6 GB), AgentMux.*
> Free up memory (close some apps) or free disk so Windows can grow its page file, then reopen.
> [ Reopen AgentMux ]   [ Details… ]

- "Details…" links to the crash-reason file / the Win10 pagefile spec.
- Reason fields (`exit_code`, `commit_free_at_death`, top consumers) flow from the host's last-breath
  struct → launcher. Top-consumer list comes from a cheap `GlobalMemoryStatusEx` + a process snapshot
  the launcher takes when it sees the host die.
- Optionally also raise a **Windows toast** (`PushNotification`-style) so the reason survives even if
  the user already walked away and dismissed the modal.

## 5. Phased plan

| Phase | Scope | Effort | Files |
|-------|-------|--------|-------|
| **P0** | **Kill the ugly box.** `SetErrorMode(+SEM_NOGPFAULTERRORBOX)` + `SetUnhandledExceptionFilter` (minimal, no parachute yet) on host + CEF subprocesses + launcher. Filter just signals launcher + returns EXECUTE_HANDLER. | small | `agentmux-cef/src/lib.rs`, subprocess entry, `agentmux-launcher/src/main.rs` |
| **P0** | **Move the native dialog to the moment of death.** Fire `show_fatal_dialog` from the launcher as soon as it classifies host exit as OOM, with the reason text — not only after the 5-min deadline. | small | `mem_supervisor.rs`, `main.rs` |
| **P1** | **Renderer-independent path.** Add `PAINT_FLOOR_MB`; below it, skip HTML recovery and go straight to launcher native dialog. | medium | `client/mod.rs` `on_render_process_terminated` |
| **P1** | **Parachute reserve** in host + launcher; free-on-fault so the last-breath handler/closeout has heap. | medium | new `agentmux-cef/src/last_breath.rs`, launcher |
| **P1** | **Abnormal-exit durable closeout** (WAL checkpoint + saga cancel) driven by launcher on OOM classification. | medium | launcher + srv ipc |
| **P2** | **Reason propagation + top-consumer snapshot + toast.** Structured reason struct host→launcher; process snapshot; optional Windows toast. | medium | host-pipe, launcher |
| **P2** | **Crash reporter** (Breakpad/Crashpad) for aggregated OOM signatures — separate effort, enables fleet-wide visibility. | large | build + host |

## 6. Non-goals
- Making Chromium survive OOM (impossible by design — see §7).
- Replacing the existing gated recovery for the *common* transient renderer crash **with memory
  available** — that HTML path stays; we only add the no-memory fallback.
- A full crash-telemetry backend (P2 stub only).

## 7. Why "just handle the allocation failure" isn't an option
Chromium "configures its memory allocators to prefer crashing rather than returning `nullptr`, so an
OOM crash can be triggered from anywhere in the code" — the OOM intercept (`OnNoMemoryInternal`)
calls `RaiseException(0xE0000008, EXCEPTION_NONCONTINUABLE)`. You cannot catch-and-continue past a
non-continuable exception, and partition_alloc won't hand back null for us to check. Hence the design
is **own the death**, not prevent it: free a reserve, suppress the OS box, flush durable state, and
explain — all from the surviving launcher.

## 8. Verification
- Force the condition: shrink the pagefile + spawn a memory balloon until commit < `PAINT_FLOOR_MB`,
  trigger a renderer OOM. Expect: **no raw Windows fault box**, no half-painted HTML, one native
  dialog with the reason, WAL truncated, sagas closed, clean relaunch.
- Confirm `SetErrorMode` is read-modify-write (doesn't clobber `SEM_FAILCRITICALERRORS`).
- Confirm the parachute is actually freed in the filter (instrument the handler).
- Confirm the dialog fires at death, not after the 5-minute deadline.

## 9. Sources
- [Chromium — Investigating OOM crashes](https://chromium.googlesource.com/chromium/src/+/main/docs/memory/oom.md) — allocators prefer crashing over null; `OnNoMemoryInternal`.
- [OOM crashes for Chromium browsers (memory-dev)](https://groups.google.com/a/chromium.org/g/memory-dev/c/mPeec9KEc74) — `0xE0000008`, allocation size in exception record.
- [The Old New Thing — Disabling the program crash dialog](https://devblogs.microsoft.com/oldnewthing/20040727-00/?p=38323) — `SEM_NOGPFAULTERRORBOX`, read-modify-write the error mode.
- [libuv suppresses Windows Error Reporting (#1327)](https://github.com/libuv/libuv/issues/1327) — practical WER/error-mode suppression.
