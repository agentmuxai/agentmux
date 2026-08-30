# Retro: fresh 0.55.21 portable build hung "Not Responding" on first launch

**Date:** 2026-08-23
**Area:** `agentmux-cef` (host/UI process) startup path, specifically the window
brings up sequence — `client::lifecycle` / `browser_pane::callbacks` — on Windows.
**Status:** live incident write-up, diagnostics captured after the fact are
incomplete (process was killed before a hang dump could be taken) — root cause
is a strong, evidence-backed hypothesis, **not a confirmed, reproduced finding**.
Flagging that distinction explicitly per this repo's own conventions (compare
[retro-keychain-timeout-retry-thread-accumulation-2026-08-22.md](retro-keychain-timeout-retry-thread-accumulation-2026-08-22.md),
which had much stronger evidence — a live `sample` thread dump — and still
labeled itself "live remediation only, not a code fix").

---

## 1. Symptom (as observed live)

A brand-new portable build (`0.55.21+gf59bb43b7`, built via `task package` from
latest `main` immediately after merging PR #2761 and PR #2768) hung at the
splash/startup screen on its first launch. The window never became
interactive.

## 2. What was confirmed

- **`tasklist //FI "PID eq 6708"` reported the process as `Not Responding`**
  (Windows' own UI-thread responsiveness check, not an inference) — this is a
  real native-layer hang, not merely "the frontend JS looks stuck to the
  user."
- **CEF's remote-debugging port (`60195`, logged at startup) never answered
  at all** — `curl` to `http://127.0.0.1:60195/json` established a TCP
  connection but received zero bytes back within a 5s timeout. Since that
  endpoint is served from the same process, this is consistent with the
  process's own event/message-handling machinery being wedged, not just the
  visible window failing to paint.
- **The host log (`agentmux-host-v0.55.21.log`) shows a completely normal
  startup sequence**, through: CEF init → IPC server bound → launcher
  handshake → 3 browser instances created (`main` top-level window, a
  window-pool prewarm window, a floating-pane-pool prewarm window) →
  `CEF initialized, entering message loop` → one navigation-focus event
  (`on_set_focus source=FOCUS_SOURCE_NAVIGATION`) on the floating-pane-pool
  browser. **Nothing from the UI thread (`ThreadId(2)`) is logged after
  that single focus event, for the entire remainder of the run** (multiple
  minutes, until the process was killed).
- **A separate background thread (`ThreadId(36)`, the memory-heartbeat
  timer) kept logging every 20s the whole time.** This rules out the whole
  *process* being suspended (e.g. by the OS scheduler, a debugger, or a full
  process freeze) — only the UI/message-pump thread stopped making forward
  progress. That's the signature of a genuine deadlock or an unbounded
  blocking wait on that one thread, not a crash, a suspend, or a slow
  computation that would eventually finish.
- **The `agentmux-srv` backend log for the same instance shows normal,
  ongoing activity** (periodic `mem_attribution` entries, no gap) — the
  backend process was healthy throughout. The hang is isolated to the
  `agentmux-cef` host/UI process.
- **Killing the specific hung PID (`taskkill /PID 6708 /T /F`) and
  relaunching the identical, unmodified build succeeded immediately** — new
  process came up `Running` (not `Not Responding`), normal window title,
  normal log progression, fully interactive. Not a systematic per-build
  defect in this artifact.
- **No corresponding Windows Event Log entries** — checked both `System` and
  `Application` logs for the exact window (2026-08-22 21:55–22:15 PT /
  2026-08-23 04:55–05:15 UTC) at Information level and above: zero matches.
  No `Application Hang` (WER) event, no driver/TDR/power event, no crash
  event of any kind.
- **No crash/hang dump exists for this incident** — `%LOCALAPPDATA%\CrashDumps`
  has several older `agentmux-cef.exe.<pid>.dmp` files (June–July), none from
  this incident. WER's formal hang-detection dialog/report needs sustained
  unresponsiveness beyond what elapsed before the process was killed, and
  `LocalDumps` (`HKLM\...\Windows Error Reporting\LocalDumps`) is not
  configured on this machine to capture hangs specifically (only unhandled
  exceptions, by default) — so no forensic memory image survived past the
  `taskkill`.
- **System load at launch time was high but not obviously pathological**:
  `mem_heartbeat` from a concurrently-running instance in the same window
  showed ~69–70% system CPU load, ~500–513 total OS processes, ~18–20 GB of
  61.6 GB physical RAM free, ~103–105 GB of 173 GB pagefile free. `muxlog ls`
  showed roughly 30 separately-tracked AgentMux dev/channel instances with
  recent activity on this machine at the time. Elevated, but RAM/pagefile
  headroom was not critically low.

## 3. What this is NOT

- **Not a regression from PR #2761 or PR #2768.** Both PRs (the pane
  block-stack reveal-gate fix and its Phase 3-4 follow-up, the two most
  recent changes in this build) touch only `frontend/**` TypeScript/SolidJS
  and `.scss` files. Zero changes to `agentmux-cef`, window/browser
  creation, CEF initialization, or any other native/Rust code in either PR.
  A frontend-only change cannot plausibly cause a native UI-thread deadlock
  that occurs before any JS-driven interaction and while the frontend
  console bridge shows no application log lines at all (see below) —
  the hang happened upstream of the frontend even getting meaningfully
  underway.
- **Not a JS/frontend error.** `muxlog fe` (frontend console bridge) for
  this instance returned zero lines — not even ordinary startup logging —
  which is consistent with the native host never getting far enough into
  page load/JS execution to produce any, rather than a JS exception (an
  exception would typically still have produced *some* prior console
  output first).
- **Not the macOS Keychain thread-leak pattern from
  [retro-keychain-timeout-retry-thread-accumulation-2026-08-22.md](retro-keychain-timeout-retry-thread-accumulation-2026-08-22.md).**
  That bug is scoped to macOS's blocking `SecKeychainFindGenericPassword`
  call inside `agentmux-srv`, accumulating leaked OS threads in the
  *backend* process specifically until *it* degrades. This incident is on
  Windows, in the *host* process, and the backend log shows no signs of
  distress. The one identity-related line in this instance's srv log
  (`no muxbus credentials stored` → `no longer auto-retrying`) is a single,
  clean give-up, not a retry loop — the exact pattern that retro identified
  as *not* a concern on its own.
- **Not a GPU driver crash/reset or power event** — checked and ruled out
  via the Windows Event Log (§2).
- **Not a full-system freeze** — the heartbeat thread's continued logging
  rules this out; only the one process's UI thread was affected.
- **Not (as far as could be checked) a disk/antivirus/OS-level stall on the
  portable folder itself** — no corresponding System/Application event, and
  the *identical* files launched cleanly seconds later from the same
  location.

## 4. Most likely explanation (unconfirmed)

The evidence points at a **transient, load/timing-sensitive stall inside
CEF's own native window/browser-creation or IPC-handshake path on the UI
thread**, most plausibly triggered or exacerbated by the unusually high
number of concurrent CEF-based processes already running on this machine at
launch time (~30 tracked AgentMux instances, each itself multi-process under
Chromium's process model, contributing to ~500+ total OS processes and ~70%
CPU load). This class of intermittent Windows CEF startup flakiness under
heavy concurrent load is exactly what this codebase's own build-time comments
already anticipate — see `Taskfile.yml`'s CEF-bundling step, which explicitly
ships the SwiftShader software-GL fallback because "the GPU process
STATUS_BREAKPOINTs at init" is a known, if intermittent, Windows failure mode
for this app, and the comment there notes hardware GL is merely "preferred
when it boots," implying it does not always boot cleanly on the first try.

This is a hypothesis consistent with all confirmed evidence above, not a
proven mechanism — **no thread/stack trace was captured for the actual hung
UI thread**, since the process was killed (a reasonable, low-risk
troubleshooting step — the instance was unusable either way) before a hang
dump could be taken. The specific blocked call was never identified.

## 5. Fix applied (live remediation only, not a code fix)

Killed the specific hung PID by ID (`taskkill //PID 6708 //T //F` — scoped to
that one process and its own child tree, never by image name, per this repo's
own multi-instance safety rule) and relaunched the identical, unmodified
build. It came up clean on the very next attempt. No code change is being
proposed from this single occurrence — there isn't enough evidence yet to
target a specific fix, and forcing one without reproduction risks fixing the
wrong thing.

## 6. Follow-up

- **If this recurs, capture a hang dump BEFORE killing the process** —
  either `procdump -ma <pid>` (Sysinternals) against the specific hung
  `agentmux-<version>.exe` PID, or Task Manager's own "Create dump file"
  right-click action, then inspect the resulting `.dmp` in WinDbg
  (`~*k` for all-thread stacks) to see exactly what the UI thread was
  blocked on. That single artifact would turn this from a hypothesis into a
  confirmed root cause.
- **Consider whether `LocalDumps` should be configured for hang capture on
  dev machines that build/run this app frequently** — would make a future
  occurrence self-diagnosing without needing to remember to `procdump`
  before killing it.
- **Consider whether this machine's very high concurrent-instance count is
  itself worth trimming** — `docs/specs/SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`
  already exists for pruning stale per-build channels; if ~30 live instances
  is typical for this dev machine rather than a one-off pile-up, running
  that pruner (or just closing unused older builds) reduces the concurrent
  CEF-process load this hypothesis blames, and would be a useful test of it
  — if hangs stop recurring after trimming concurrent instance count, that's
  meaningful corroborating evidence even without a dump.
- **Not reproduced on the very next launch** — given the transient nature and
  lack of a definitive mechanism, this is being filed as a data point rather
  than escalated further right now. If it recurs (especially reproducibly,
  or independent of concurrent-instance load), that would justify deeper
  investigation — e.g. deliberately reproducing under WinDbg attached from
  launch, or bisecting whether it's specific to the pane-pool/floating-pane
  prewarm path (the last code area logged before the hang) versus generic
  CEF init flakiness.
