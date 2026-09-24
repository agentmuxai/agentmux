# SPEC: Universal install dialog — plain-language steps by default, full console under "Details"

**Date:** 2026-09-23
**Status:** proposed
**Author:** AgentO
**Supersedes (UI parts of):** `SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md` §5 (Details open by default in the provider modal)
**Builds on:** `SPEC_AGENT_INSTALL_STAGE_2026_05_17.md` (the `install.start` / `install_chunk` machinery and its never-shipped Phase β step model), `SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md`, `SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md`

---

## 1. Problem

Installing Pi from the agent picker produced a dialog that "scrolled far". Every
install surface in AgentMux shows the raw installer console as its main
content. A user who wants to know "is it working, and how far along is it?" has
to read npm internals.

### 1.1 Why the Pi dialog scrolls, confirmed in source

Four causes stack up. The first two are the likely direct trigger.

1. **Verbose npm output.** `spawn_install_task`
   (`agentmux-srv/src/server/install_handlers.rs:558-567`) always runs
   `npm install … --progress=false --loglevel=verbose`. Pi
   (`@mariozechner/pi-coding-agent`) has a large dependency tree, so this emits
   thousands of `fetch` / `reify` lines. npm writes verbose output to stderr,
   and `AgentInstallModal.tsx:112-119` paints every stderr line red. The result
   is a fast-scrolling wall of red text that reads as failure even when the
   install is healthy.
2. **Panel scroll position leaks across the prereq → install hand-off.** Pi
   declares Node and npm as `systemPrereqs`. When either is missing, the flow is
   the prereq modal first, then `modalLayer.replace(install-agent)`. `replace`
   keeps the same `<Modal>` shell and swaps only its content
   (`ModalLayer.tsx:130-138`). The scroll container is `.modal-panel` itself
   (`modal.scss:87-91`, `overflow: auto`), so a prereq modal that grew and was
   scrolled down to reach "Launch anyway" hands the install modal an
   already-scrolled panel.
3. **Minimum sizes that do not fit a pane.** Agent-picker modals are pane
   scoped, capped at `calc(100% - 48px)` of the pane (`modal.scss:151-154`). The
   install body sets `min-height: 320px`, `min-width: 560px`, and a
   window-relative `max-height: 60vh` (`_install-modal.scss:15-18`). In a short
   or narrow pane the whole panel scrolls, and the footer buttons fall out of
   view. Between 400 and 600px wide the 560px minimum also forces horizontal
   overflow.
4. **Focus lands in the terminal.** On open, `<Modal>` focuses the first
   focusable element (`modal.tsx:300`). The only match is xterm's hidden helper
   textarea, which can scroll the panel down to the terminal and keep dragging
   it as the cursor moves.

### 1.2 The wider inconsistency

The frontend has six install or upgrade surfaces, each with its own progress
presentation:

| Surface | File | Progress today |
|---|---|---|
| Provider CLI install modal (Claude, Codex, Gemini, Pi, Qwen, …) | `view/agent/components/AgentInstallModal.tsx` | xterm console in a `<details>` that defaults **open** |
| Missing-prereq modal | `view/agent/components/AgentPrereqModal.tsx` | list of missing tools, each expanding an inline installer; body has no max-height |
| Inline system-tool installer (git, Node, npm, Python) | `view/toolchain/SystemToolInstallInline.tsx` | `<pre>` log in a `<details>` that defaults **closed**; no cancel |
| Automatic CLI install at launch | `view/agent/flows/launch-flow.ts:214-265` | batch of raw lines dumped into the agent's shell log after the process exits |
| Silent CLI install from login panels | `ClaudeLoginPanel.tsx:153`, `PreLaunchAuthPanel.tsx:342` | none; up to 120 s of spinner, then an error string |
| Managed binary tools (`/tools install` jq, rg) | `commands/global/tools.ts:92` | none until a final summary message |
| App self-update and migrations | `statusbar/UpdateStatus.tsx`, `MaintenanceSection.tsx` | text status only; migrations are the one real step list |

The backend has three unrelated progress channels (`install_chunk`,
`install_progress`, and the CEF updater and migration events). None of the
install paths emits a step or phase. The subscribe-and-parse handler is copied
between the provider modal and the inline installer.

## 2. Goals

1. **One install experience everywhere.** Every place AgentMux installs,
   upgrades, or sets something up renders the same component with the same two
   layers.
2. **Layer 1, the default: a short list of plain-language steps.** Each step is
   one line a non-developer understands, such as "Download packages" or "Check
   it runs", with a status icon. The user sees where they are and what is left
   without reading a log.
3. **Layer 2, "Details": the full console.** Collapsed by default, one click
   away, bounded in height, auto-following, copyable. Nothing is hidden from a
   user who wants it.
4. **The dialog never scrolls as a whole during a normal install.** Header,
   step list, and footer buttons stay in view at every supported pane size.
5. **Failures explain themselves in Layer 1.** A failed step says what went
   wrong in plain words and what to do next, and Details opens automatically at
   the first error.
6. **Prereqs and the product install are one plan.** Installing Pi on a machine
   without Node shows one dialog: "Install Node.js", then "Install Pi". No
   modal hand-off.

## 3. Non-goals

- Changing *what* gets installed, pin versions, install locations, or the
  package managers used. This spec covers presentation and progress reporting
  only.
- Bootstrapping a missing package manager (Homebrew, winget). That remains open
  in `SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md`.
- Background installs that survive closing the dialog. See §11.
- Real percentage progress for npm. npm does not expose a reliable total, so
  steps are the unit of progress. A step may show a count when one is known.

## 4. The two layers

### 4.1 Layer 1: step list (default view)

```
┌─────────────────────────────────────────────┐
│ ⬢  Install Pi                     v0.73.1  │
│    About a minute. Needs an internet        │
│    connection.                              │
├─────────────────────────────────────────────┤
│  ✓  Check requirements                      │
│  ✓  Install Node.js                  22.11  │
│  ◐  Download packages          214 fetched  │
│  ○  Set up files                            │
│  ○  Check Pi runs                           │
│                                             │
│  ▸ Details                                  │
├─────────────────────────────────────────────┤
│  0:42                    [Cancel]           │
└─────────────────────────────────────────────┘
```

Rules:

- **Header.** Product icon, "Install <name>", version pill, one line saying
  roughly how long it takes and what it needs. The existing
  `SPEC_INSTALL_MODAL_VERSION_DISPLAY` version pill stays.
- **One row per step.** Status icon, label, and an optional right-aligned
  *hint*: a count, a version, or a short fact. Labels are verbs in plain
  language. No package names, flags, or paths in Layer 1.
- **Status icons.** `○` pending, `◐` active with a subtle animation, `✓` done,
  `✗` failed, `–` skipped with a hint such as "already installed". Colour is
  never the only signal.
- **Active step subline.** The active step may show one muted subline that
  updates live, such as "Downloading typescript". It is a single line with
  ellipsis and never grows.
- **Elapsed time** sits in the footer, left-aligned. The indeterminate progress
  bar is removed. The step list is the progress indicator.
- **Success.** All rows ticked, header reads "Pi is installed", primary button
  "Continue". Where a restart is required, the last row says so and the primary
  button offers it.
- **Failure.** See §7.

### 4.2 Layer 2: Details (full console)

- A disclosure row, "Details", below the step list. **Collapsed by default on
  every surface.** This reverses the provider modal's current default-open
  choice. Its original reason, "otherwise the body is empty", no longer applies
  because Layer 1 is the body.
- Expanded, it shows the **complete** raw output of every step in order. A dim
  `$ <command>` echo line separates steps.
- Bounded height: at most 40% of the dialog's available height, with a floor of
  about eight lines. It scrolls internally and never pushes the footer out of
  view.
- Sticks to the bottom while output streams, unless the user has scrolled up.
  Reuse the proven stick-to-bottom logic from `SystemToolInstallInline.tsx`
  (40 px threshold, re-check inside the rAF, re-sync on open).
- Actions in the Details header: **Copy all** and **Save log…**.
- Colour: stderr is **not** painted red wholesale. Only lines the classifier
  marks as errors or warnings (§6.3) get the error or warning colour. npm
  verbose output on stderr renders in the normal console colour.
- Renderer: the console is a read-only log, not an interactive terminal. It
  uses a lightweight virtualised line list (ANSI colour support, monospace,
  selectable text) rather than xterm.js. This removes the focus-trap problem in
  §1.1 item 4, the FitAddon sizing work, and the `ModalLayer`
  mutation-observer churn xterm's DOM renderer causes. A capped buffer of 20,000
  lines with a "trimmed N earlier lines" marker bounds memory; Save log writes
  the untrimmed file the backend streams to disk (§6.5).
- The user's open or closed choice is remembered per install kind for the
  session.

### 4.3 Layout rules (fixes §1.1 items 2–4)

1. `.modal-panel` does not scroll for install dialogs. The dialog is a flex
   column: header and footer fixed, body `flex: 1; min-height: 0; overflow: auto`.
   The body is the only scroll container.
2. The step list is expected to fit. It is short by construction, capped at
   about eight steps.
3. Sizes are relative to the modal's container, never the window. Remove
   `min-width: 560px` and the `60vh` cap; use `width: min(640px, 100%)` and
   container queries on `modal-mount`. Below 400 px wide, hints drop under
   their label.
4. Opening or replacing dialog content resets the body's `scrollTop` to 0. This
   applies to `modalLayer.replace` for **every** modal kind, not just
   installs, since the leak is a general bug.
5. Initial focus goes to the primary footer button. Details is never
   auto-focused.

## 5. One component, one model

### 5.1 Frontend

New module `frontend/app/element/install/`:

- **`InstallSession`** — a reactive model: `plan: InstallStep[]`,
  `log: LogLine[]`, `state: idle | running | done | failed | cancelled`,
  `elapsedMs`, `error`, `canCancel`. It subscribes to one session scope and is
  the **only** code that parses install events. It replaces the handlers
  duplicated in `AgentInstallModal.tsx:171-193` and
  `SystemToolInstallInline.tsx:226-244`.
- **`<InstallProgress session>`** — Layer 1 plus Layer 2 with no chrome. Used
  inline, for example in the Toolchain pane rows.
- **`<InstallDialog request>`** — `<InstallProgress>` wrapped in the standard
  modal chrome, header, and footer, with the §4.3 layout. Used by every modal
  install.
- **`<InstallConfirm>`** — the pre-start consent state for anything that
  needs it: what will be installed, the command preview, and the elevation
  warning. Today this lives only in `SystemToolInstallInline`'s idle phase. It
  becomes a shared state of the same dialog, shown before step 1 and applied
  uniformly.

### 5.2 Surface migration

| Surface | Becomes |
|---|---|
| Provider CLI modal | `<InstallDialog>` with an npm-CLI plan |
| Prereq modal + provider modal | **one** `<InstallDialog>` whose plan prepends a step per missing prereq (§8). `AgentPrereqModal` keeps only its "not installable here, open the download page" rows and "Launch anyway". |
| Toolchain pane rows | inline `<InstallProgress>` |
| Launch-flow auto install (`resolvecli`) | the agent pane's launch view renders `<InstallProgress>` live instead of dumping lines into the shell log. The shell log still receives the raw lines. |
| Login-panel silent installs | show `<InstallProgress>` inline in the login panel instead of a spinner |
| `/tools install` | inline `<InstallProgress>` in the command result, one plan for all requested tools |
| App self-update | `<InstallProgress>` in the maintenance section, steps "Download update", "Verify", "Ready to restart" |
| Migrations | already a step list; re-rendered with the same row component so it looks the same |

## 6. Wire protocol: structured steps

### 6.1 New events on the existing `install_chunk` channel

Keep `install_chunk`, scope `install:<sessionId>`, and add two ops. Old clients
ignore unknown ops, and existing `line` and `done` events are unchanged.

```ts
// Every event below also carries `seq: number`, monotonically increasing per
// session, so a client can merge live events with a snapshot (§6.4).

// Sent once, right after start, and again if the plan changes.
{ op: "plan", sessionId, seq,
  steps: { id: string; label: string; unit: string }[] }  // unit: §7.1

// Sent on every step transition, and optionally for live hints.
{ op: "step", sessionId, seq, id: string,
  status: "pending" | "active" | "done" | "failed" | "skipped",
  hint?: string,        // right-aligned short text: "214 fetched", "22.11"
  subline?: string,     // live one-liner under the active step
  error?: InstallError }

// Existing, extended: every line now carries the step it belongs to.
{ sessionId, seq, line, stream: "stdout" | "stderr", step?: string }
```

These get real Rust structs and generated TypeScript types. Today the payload
is untyped JSON and the consumer takes `event: any`. The `pending` status
exists so a Retry can visibly reset the steps of a unit being rerun (§7.1).

### 6.2 Standard plans

The backend owns the plan, so labels stay consistent everywhere.

**npm provider CLI** (`install.start`, and `resolvecli` when it installs):

| id | Label | Derived from |
|---|---|---|
| `requirements` | Check requirements | Node and npm present; the `which npm` pre-check `resolvecli` already runs, now also run by `install.start` |
| `download` | Download packages | npm `http fetch` lines; hint is the running fetch count |
| `setup` | Set up files | npm `reify` and `extract` lines |
| `scripts` | Run setup scripts | npm lifecycle lines (`postinstall`); skipped if none ran |
| `verify` | Check <Name> runs | bin shim exists and `<cli> --version` succeeds; hint is the version. `install.start` gains the `--version` check `resolvecli` already has. |

**System tool** (winget, brew, pkexec with a Linux package manager):

| id | Label |
|---|---|
| `prepare` | Get ready |
| `permission` | Ask for permission (only when `needsElevation`) |
| `install` | Install <Tool> |
| `verify` | Check <Tool> is available |
| `restart` | Restart AgentMux to finish (only when the PATH re-check fails; replaces today's free-text note) |

**Managed binary** (`tool_store::install_tool`): `download`, `checksum`
("Check the download is intact"), `unpack`, `finish`. The phases already exist
as separate code blocks and only need to emit events. `installtool` gains a
session id and streams like the others.

**App update:** `download`, `verify`, `ready`.

### 6.3 Classifying output

A backend classifier tags each line and each failure. It lives next to the
spawn code so `resolvecli`, `install.start`, and system installs share it.

- **Step transitions** come from command boundaries plus npm's verbose line
  prefixes. Derivation is best-effort: if a pattern never appears, the step
  still completes when the next one begins or the process exits. A missed
  pattern can make a step look instant but can never make the plan wrong.
- **Error categories** with a plain-language message and a next action:

| Category | Detected by | Layer 1 message | Action |
|---|---|---|---|
| `network` | `ENOTFOUND`, `ETIMEDOUT`, `ECONNRESET`, HTTP 5xx | "Couldn't reach the package server." | Retry |
| `permission` | `EACCES`, `EPERM`, declined UAC or polkit (detection per OS in §13.3) | "AgentMux wasn't allowed to write the files." | Retry, or open Details |
| `disk` | `ENOSPC` | "The disk is full." | — |
| `missing_prereq` | spawn fails for `npm`, `brew`, `pkexec`, `winget` | "<Tool> is needed first." Names the executable that actually failed to spawn, never a hard-coded Node.js. | If AgentMux can install that tool (Node.js, npm), add its step and offer to install it. If it can't, as with Homebrew, pkexec, or winget (see §3), offer "Open download page" for that tool. |
| `not_on_path` | exit 0 but verify fails to find the binary | "Installed, but AgentMux can't see it yet." | Restart AgentMux |
| `version_check` | `--version` fails or times out | "Installed, but <Name> didn't start." | Open Details |
| `cancelled` | user cancel | "Cancelled. Nothing was left behind." | — |
| `unknown` | anything else | "Something went wrong while <step label>." | Retry, open Details |

`InstallError` is
`{ category, message, action?, missingTool?: string, firstErrorLine?: number }`.
`missingTool` is set for `missing_prereq`, and the Layer 1 message and action
are rendered from it. `firstErrorLine` lets Details open scrolled to the first
line the classifier marked as an error.

### 6.4 Session state snapshot

A bounded event replay cannot rebuild step state. A Pi install emits thousands
of line events, so with `persist: 1024` the initial `plan` and the early `step`
transitions are evicted long before the install ends. A dialog that remounts
mid-install would get the last lines but no plan and no ticks.

So step state does not depend on replay:

- The backend keeps an authoritative **session snapshot** for every live or
  recently finished session:
  `{ sessionId, state, plan, steps: {id, status, hint?, subline?, error?}[], startedAt, lastSeq, logPath }`.
- A new RPC, `install.status { sessionId }`, returns that snapshot plus the
  last 500 log lines.
- Every `install_chunk` event gains a monotonically increasing `seq`.
- On mount, `InstallSession` subscribes first, buffering events. It then calls
  `install.status`, applies the snapshot, and drops buffered events with
  `seq <= lastSeq`. After that it applies live events. This is correct however
  many events were evicted.
- `persist: 1024` stays only as a convenience for the log tail. Nothing
  correctness-critical relies on it.
- Snapshots are kept until the session is dismissed, or for 10 minutes after
  it ends, whichever is later.

### 6.5 Log retention

The full log is **streamed to disk as it arrives**, never reconstructed later
from memory, so an untrimmed log always exists no matter how much output
there is:

- Path: `<data_dir>/logs/install/<provider-or-tool>-<timestamp>.log`, opened
  when the session starts and appended line by line. `logPath` is part of the
  session snapshot.
- On failure the file is kept.
- On success the file is deleted when the session is dismissed, unless the
  user chose Save log, which copies it to a location they pick.
- A sweep on startup removes install logs older than 7 days and keeps at most
  the newest 20.
- In-memory buffers are for display only. The frontend view is capped at
  20,000 lines (§4.2), and the backend keeps a 500-line tail for
  `install.status`. Neither is used to produce a saved log.

### 6.6 Unifying the three channels

- `resolvecli` stops batching output after exit and stops using the separate
  `install_progress` event. It creates an install session, emits the standard
  plan and live events on `install:<sessionId>`, and returns the session id
  early through a new optional `onSession` callback event on
  `block:<blockId>`. On Windows, `resolvecli` does not stream today. §13.1
  explains why one batch per step is not acceptable there and what replaces it.
- `install_progress` is kept for one release for old frontends, then removed.
- App update and migrations stay on their CEF events. A small adapter maps them
  into an `InstallSession` so they render with the same component.

## 7. Failure experience

- The failed row shows `✗`, its label, and the category message as a subline.
- A one-line action area under the list offers the category's action: Retry,
  Install <Tool>, Restart AgentMux, or Open download page.
- Details **opens automatically** on failure, scrolled to `firstErrorLine`.
- Footer: **Close** and the primary action. **Copy log** is always in the
  Details header, so a user can paste a failure into a bug report.

### 7.1 What Retry reruns

Retry resumes only at a real **command boundary**, never at a step derived by
the classifier.

- Each plan groups its steps into **units**, and each unit is exactly one
  process. In the npm plan, `download`, `setup`, and `scripts` are phases
  inside a single `npm install`, so together they form one unit. `verify` is a
  separate unit. In a chained plan (§8), each prereq install is its own unit.
- Retry reruns the failed unit **from its start**. For npm that means the
  whole `npm install` command. The existing rollback deletes the provider's
  partial directory first, so lifecycle scripts always run against a clean
  tree. AgentMux never assumes package lifecycle scripts are idempotent.
- When a unit is rerun, every step inside it resets to pending. Only units that
  finished before the failed one keep their ticks. For example, if Pi's
  `scripts` phase fails, "Install Node.js" keeps its tick, while "Download
  packages", "Set up files", and "Run setup scripts" all go back to pending.
- A package-manager unit, such as winget, brew, or apt, is rerun as a whole.
  Those tools are designed to be safely re-invoked after a failed run.

## 8. Chained plans: prereqs plus product

`AgentPicker` today opens the prereq modal, then replaces it with the install
modal. It becomes:

1. `AgentPicker` asks the backend for a **plan** for installing the agent's
   provider: `install.plan { providerId }` returns the prereq steps still
   needed and the CLI steps.
2. One `<InstallDialog>` shows the combined list. Prereq steps come first, for
   example "Install Node.js" (with the permission step when elevation is
   needed), then the CLI's own steps.
3. `<InstallConfirm>` shows once, before anything runs, listing every system
   change and the elevation warning if any step needs one.
4. The backend runs the chain as one session: `install.start` accepts a
   `prereqs: string[]` field and runs system installs first. If a prereq ends
   in `not_on_path`, the chain stops at a `restart` step, since npm will not be
   found until AgentMux restarts, and the dialog says so plainly.
5. Prereqs AgentMux cannot install itself stay as a row with an "Open download
   page" action and a "Check again" button. This keeps today's link-only
   behaviour for unsupported tools.

## 9. Cancel and dismissal

- One policy everywhere: **Cancel is available while a step is cancellable.**
  npm steps are cancellable and roll back as today. Package-manager steps,
  such as winget, brew, or apt, are not, because interrupting them can corrupt
  system state. That is the reasoning in the toolchain spec. During those steps
  the Cancel button is disabled with the tooltip "Can't stop <Tool> safely
  mid-install".
- Closing the dialog with ESC or the backdrop during a non-cancellable step
  asks for confirmation and lets the step finish in the background. The
  surface that opened it shows the result when it lands.
- Success always calls the caller's `onInstalled`, however the dialog was
  closed. This keeps the existing codex P2 fix from PR #895.

## 10. Implementation phases

Each phase ships on its own and is useful alone.

1. **Layout and default fixes, frontend only.** §4.3 layout rules,
   `replace` scroll reset, primary-button focus, stop painting all stderr red,
   and Details collapsed by default. This alone fixes the reported Pi symptom.
   The step list comes from a **frontend** line classifier over today's
   unstructured `install_chunk` lines using the §6.2 npm plan, so the layer
   split ships before any backend work.
2. **Shared component.** `InstallSession`, `<InstallProgress>`,
   `<InstallDialog>`, `<InstallConfirm>`, and the virtualised log. Migrate the
   provider modal, the inline system installer, and the Toolchain pane.
3. **Backend steps.** `plan` and `step` ops with `seq`, typed payloads, the
   shared classifier, error categories including `missingTool`, the
   `install.status` snapshot RPC, logs streamed to disk, unit-based Retry, and
   the `--version` verify in `install.start`. The frontend classifier from
   phase 1 becomes a fallback for old backends.
4. **Chained plans.** `install.plan`, `prereqs` on `install.start`, and removal
   of the prereq-to-install modal hand-off.
5. **Remaining surfaces.** `resolvecli` streaming, login panels, `/tools
   install`, app update, migrations re-render, and retirement of
   `install_progress`.

## 11. Open questions

1. **Background installs.** Should closing the dialog during an npm install
   continue it in the background with a status-bar indicator, instead of
   cancelling? This spec keeps today's cancel-on-close for npm steps.
2. **Time estimate in the header.** Show a static per-provider estimate, a
   rolling median from past installs on this machine, or nothing?
3. **Verbose npm output.** Keep `--loglevel=verbose` so Details is complete and
   step derivation has signal, or drop to `http` to shrink the log? This spec
   keeps verbose, since Layer 1 now hides it. Whichever is chosen, both npm
   paths must use it (§13.5).
4. **Kimi and other non-npm providers.** Their install hint (`pip install
   kimi-cli`) is never run. Should they get a real plan through the system
   installer, or stay link-only?
5. **Dead CEF installer.** `agentmux-cef/src/commands/providers.rs:566`
   (`install_cli`) is unused and installs to a different directory. Remove it
   as part of phase 5?

## 12. Testing

- **Unit.** The line classifier over recorded npm verbose logs, including a
  real Pi install, a network failure, and an `EACCES` failure. Step derivation
  when patterns are missing. `InstallSession` state transitions.
- **Remount after eviction.** Start a session that emits more than 1,024
  events, mount a fresh `InstallSession` afterwards, and assert that the plan,
  ticks, and active step match the backend snapshot, with no duplicated or
  missing lines at the `seq` boundary.
- **Retry.** Fail the npm unit during its `scripts` phase in a chained plan.
  Assert that Retry deletes the partial provider directory, reruns
  `npm install` from the start, resets all three npm steps to pending, and
  keeps the prereq unit's tick.
- **Logs.** Produce more than 5 MB of output on a successful install, then
  Save log, and assert that the saved file is byte-identical to the full
  stream. Assert that the file is deleted on dismissal when not saved.
- **Missing prereq.** Simulate a spawn failure for `brew` and for `npm`.
  Assert that each message names its own tool, that the npm case offers to
  install Node.js, and that the brew case offers only its download page.
- **Component.** Default view shows steps with Details collapsed; failure opens
  Details at the first error; stick-to-bottom behaviour; Copy all.
- **Layout.** Render the dialog in pane-scoped modals at 360×480, 480×600 and
  800×900 and assert that the footer is visible, the panel has no scrollbar,
  and there is no horizontal overflow. Assert that `replace` from a scrolled
  prereq modal yields `scrollTop === 0`.
- **Manual.** Install Pi on a machine with Node and without Node, on macOS,
  Windows, Ubuntu (apt), and Fedora (dnf). Cancel mid-download. Unplug the
  network mid-download. Confirm the dialog never scrolls as a whole and every
  failure names its cause in Layer 1. Platform-specific cases are in §13.6.

## 13. Platform notes

The dialog, plans, error categories, and log files are the same on every OS.
The places where the platforms differ are listed here, because each one can
silently break the Layer 1 promise on one OS while it holds on the others.

### 13.1 Windows: npm output must stream

Layer 1 only moves if lines arrive while npm runs. On Windows that does not
happen today on the `resolvecli` path:

- `cli_handlers.rs` runs `cmd /C npm install …` and collects output with
  `.output()` after exit. Its comment says pipe streaming does not receive data
  from `cmd.exe /C` batch children.
- `download`, `setup`, and `scripts` are phases of one `npm install` process
  (§7.1). "One batch per step" therefore means Layer 1 sits on "Download
  packages" for the whole install, then ticks everything at once. That is the
  Pi experience this spec exists to fix, so it is not acceptable.
- The `install.start` path starts `npm.cmd` with piped output. Rust's standard
  library runs `.cmd` files through `cmd.exe`, so it may hit the same problem.
  Nobody has checked whether it streams on Windows.

Required change: on Windows, both paths start npm without the batch shim.
They run `node <npm-dir>/node_modules/npm/bin/npm-cli.js install …`, resolving
`<npm-dir>` from the `npm.cmd` found on PATH. This is the command `npm.cmd`
itself runs. It also removes the `cmd.exe` quoting workaround (`raw_arg`) that
`cli_handlers.rs` needs today.

**Unverified.** It has not been checked that `node` started directly streams
through a pipe under `CREATE_NO_WINDOW`, or that the `cmd.exe` claim in
`cli_handlers.rs` still holds. Check both on a Windows machine before any
phase is called done on Windows. Phase 1's frontend classifier depends on
streaming too. If direct `node` does not stream either, fall back to a ConPTY for the
npm unit and treat its output as one stream.

### 13.2 Windows: pick up PATH changes without a restart

A running process never sees PATH changes made after it started. On macOS and
Linux this rarely matters here. Homebrew's bin directories are in the
well-known directories `toolchain_path.rs` adds at launch, and the Linux
package managers AgentMux drives (apt-get, dnf, pacman, zypper, apk) install
into `/usr/bin`. On Windows, `toolchain_path.rs` keeps the inherited
PATH unchanged, and winget's Node and Git installers add their directories to
the registry PATH. So nearly every Node install through winget ends at the
`restart` step (§6.2).

Required change: after a system install succeeds on Windows and the PATH
re-check fails, re-read the Machine and User `Path` values from the registry,
expand environment variables in them, and merge any new entries into the
server's PATH. Also pass the merged PATH to processes started afterwards,
including npm in a chained plan (§8). Run the re-check again. Show `restart`
only if that second check fails too.

### 13.3 Detecting a declined permission prompt

A user who says no to the OS prompt must see the `permission` message, not
`unknown`.

| OS | Mechanism | Declined looks like |
|---|---|---|
| Linux | `pkexec` | Exit 126 when the user dismissed the auth dialog. Exit 127 when not authorized or on error, so 127 maps to `permission` only if the output has no other error. |
| Windows | winget raises the installer's own UAC prompt | The exit code winget returns for a declined UAC prompt is not documented here. Record it on a real machine for the Node and Git installers, then map it. Until then, match the declined-elevation text in winget's output. |
| macOS | brew never elevates (§6.2) | Not applicable. `EACCES` from brew maps to `permission` as usual. |

### 13.4 Linux: no polkit agent

`pkexec` needs a running polkit authentication agent. Minimal window managers
and some server desktops have none, so `pkexec` fails at once. The existing
`available` pre-check only confirms that `pkexec` exists. Treat an immediate
127 before any package-manager output as `missing_prereq` with
`missingTool: "a polkit authentication agent"`. Keep the existing
copy-command fallback as the action, showing the `sudo <package manager>
install …` command, since AgentMux cannot raise the prompt itself.

### 13.5 One npm log level on both paths

`install.start` runs npm with `--loglevel=verbose`. `resolvecli` runs it with
`--loglevel=http`. At `http` npm prints no `reify` or `extract` lines, so the
`setup` step would advance differently depending on which path started the
install. Both paths use one level, set in one place (see §11 question 3). This
is not an OS difference, but it shows up the same way: the same install looks
different on different surfaces.

### 13.6 Platform test cases

Add these to §12:

- **Windows streaming.** Install Pi through `install.start` and through
  `resolvecli`. Assert that Layer 1 moves from `download` to `setup` while
  npm is still running, not at exit.
- **Windows PATH.** On a machine without Node, install Node through winget in
  a chained plan. Assert that the chain continues to `npm install` without a
  `restart` step.
- **Declined prompt.** Decline UAC on Windows and polkit on Linux. Assert that
  Layer 1 shows the `permission` message.
- **No polkit agent.** On Linux with no polkit agent running, assert the
  `missing_prereq` message and the terminal command in Details.
- **Log level.** Assert that both npm paths pass the same `--loglevel`.
