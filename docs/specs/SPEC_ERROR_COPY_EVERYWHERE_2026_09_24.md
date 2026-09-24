# SPEC: every error surface can be copied, errors and traces in one click

**Date:** 2026-09-24
**Status:** proposed — nothing implemented. Inventory verified against `main` @ `b96df30cb`.
**Author:** agentx
**Trigger:** Repo owner, after a 0.57.2 portable opened on "AgentMux lost its
connection to the host": *"we want to make sure all error areas have a copy
button, so users can quickly copy errors and traces, put them at the best
places."*
**Builds on:** `SPEC_COPY_BUTTON_FALSE_POSITIVE_FIX_2026_08_10.md` (#2535: the
clipboard path `CopyButton` uses, and its lesson that a check mark isn't proof
of a copy), `SPEC_ERROR_CATALOG_2026_05_17.md` (the AMX error codes the
payload carries), and `SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` (the agent
failure model the agent rows render).

---

## 1. Problem

When something fails, the text a user needs for a bug report is usually on
screen but hard to take away. Of about 45 user-visible error surfaces
(§5), only 5 have a real copy button. Another 3 copy through a gesture nothing
tells you about: click the whole toast, or highlight the text. The rest need
the text selected by hand, which some of them block. The failure a user hits
most, the agent pane's failure row, can't be copied at all, and its summary
line can't even be selected (`PaneRow.scss:30`).

The incident that prompted this shows the whole chain:

- The renderer's first request to srv failed 1.5 s after srv started
  (`TypeError: Failed to fetch`, host log 19:59:08.086).
- `initHostMux` then showed the connection-lost card. Its "Technical details"
  (`error-display.ts:329-347`) has no copy button.
- The console line logged the error as `Initialization failed: {}`, because
  the `Error` object serialized to nothing.
- `showStartupError(String(error))` (`app-init.ts:438`) drops the stack.

The detail that would explain the failure was lost twice before anyone could
copy it.

## 2. Goals and non-goals

**Goals**
- G1. Every error surface has a visible, labeled way to copy the error. Where
  the surface has more than a message (a stack, a stderr tail, exit codes,
  ids), it copies all of it, not just what's visible.
- G2. One shared component and one text format, so a pasted report reads the
  same wherever it came from.
- G3. Copy works where the host bridge may be dead: the connection-lost card,
  and the CEF crash, hang and low-memory pages.
- G4. Crash-class surfaces also offer **Copy diagnostics**: the error plus the
  environment and log locations a developer asks for first.
- G5. Nothing secret reaches the clipboard (§4.5).

**Non-goals**
- Uploading reports, or a bug-report form. This is copy only.
- Changing what errors say. That's `SPEC_ERROR_CATALOG`.
- Native OS dialogs (§4.6).

## 3. What exists to build on

| Piece | Where | Use it for |
|---|---|---|
| `writeText` | `frontend/util/clipboard.ts:10` → IPC `write_clipboard` (`agentmux-cef/src/commands/clipboard.rs:15`) | The clipboard write. `navigator.clipboard` is blocked in CEF; #2535 fixed this path |
| `<CopyButton>` | `frontend/app/element/copybutton.tsx:15` | Icon button with copied / failed states. Used by `blockframe.tsx:943`, `markdown-codeblock.tsx:65` |
| `CopyableErrorMessage` | `view/accounts/AgentMuxConnectPanel.tsx:165` (file-private) | The shape wanted for inline error text. Export it and generalize it |
| Copy on highlight | `block/BlockErrorBoundary.tsx:160-181` | Its fallback order: `execCommand` first, then IPC, for surfaces whose bridge may be down |
| Install page "Copy path" | `agentmux-cef/src/commands/window/creation.rs:~220`, test at :1105-1118 | The rule for CEF `data:` pages: wire the handler with `addEventListener` in a `<script>`, never an inline `onclick` |
| Diagnostics sources | `getAboutModalDetails` (`cef-api.ts:102`), `get_host_info` (`platform.rs:234`), `getBackendInfo`, `backendDeathInfoAtom`, `statusbar/InstancePanel.tsx:410` (already copies version / channel / build) | The diagnostics block (§4.3) |
| Reveal in Explorer | `reveal_in_file_explorer` (`ipc.rs:294`, `cef-api.ts:466`) | "Reveal logs" (§4.3) |

## 4. Design

### 4.1 One text format: the error report

All copy actions put plain text on the clipboard. The format pastes cleanly
into GitHub, Slack, email and an agent's composer:

```
AgentMux error: <title>
<message>
Code: <AMX code / HTTP status / exit code / signal, whichever apply>
Where: <surface> · pane <short block id> · <view type or provider>
When: <ISO-8601 local time>

Details:
<detail text, stderr tail, stack: full, never truncated to what's on screen>
```

- Lines with nothing to say are omitted.
- `Details` keeps the original line breaks.
- The builder is one function, `formatErrorReport(fields)` in
  `frontend/app/errors/error-report.ts`, with a unit test per surface type.
  Surfaces pass fields; they never format the text themselves.

### 4.2 One component: `<CopyErrorButton>`

It builds on `<CopyButton>` and takes a `report: () => string` (lazy, so a big
stack is only formatted on click). Two presentations:

- **`variant="action"`**: a text button, "Copy error" (or "Copy details" when
  it includes a stack or stderr). For surfaces that already have an action
  row: Retry, Restore, Reload, Log in.
- **`variant="icon"`**: the copy icon, shown on hover and focus. For inline
  rows (transcript error rows, form errors, toasts).

Behavior:
- **Feedback.** On success the label or icon becomes "Copied ✓" for 2 s. On
  failure it becomes "Copy failed". The text also becomes selected, with
  "Press Ctrl+C" beside it, so the user is never left with nothing.
- **Transport order.** `writeText` (IPC) first. If that throws, a hidden
  `<textarea>` + `document.execCommand("copy")`, inside the same click. This
  is `BlockErrorBoundary`'s approach, in reverse order, because IPC is the
  path #2535 verified. Surfaces whose bridge is known to be down pass
  `transport="dom"` to go straight to `execCommand`.
- **Keyboard.** It's a real `<button>` with an `aria-label`. On inline rows
  the icon shows on `:focus-within`, not only on hover.

### 4.3 Copy diagnostics

Crash-class surfaces (§5, "D") offer a second action, **Copy diagnostics**.
It's the §4.1 report followed by:

```
Diagnostics:
AgentMux <version> (<build label>, <git hash>) · channel <channel> · <platform>/<arch> · CEF <version>
Backend: pid <pid>, up <duration> | died <time>, exit <code>, signal <signal>
Window <label> · tab <id> · pane <id>
Logs: <host log path>
      <srv log path>
```

- Every field comes from something that already exists (§3) except the log
  paths.
- **New IPC `get_log_paths`** (`agentmux-cef/src/commands/platform.rs`)
  returns the current host log, srv log, launcher log and `cef-debug.log`
  paths. They're already known at `agentmux-cef/src/logging.rs:10-23`,
  `agentmux-srv/src/bootstrap.rs:312` and
  `agentmux-launcher/src/logging.rs:18`.
- Crash-class surfaces also get **Reveal logs**, which opens the log folder
  via `reveal_in_file_explorer`.
- Diagnostics never include log contents, only paths. A log can hold more
  than the user means to share, and the paths are what's needed to ask.

### 4.4 Placement rules

1. **Next to the surface's primary action**, second in reading order, as
   `variant="action"`. A user who hits an error looks at the buttons first.
2. **Never only inside a collapsed section.** If a surface hides detail
   behind "Details" or "Technical details", the copy button stays visible
   while collapsed and still copies the full detail.
3. **Inline rows get the icon at the row's end**, visible on hover and focus.
   They also get a "Copy error" entry in the row's right-click menu, where
   one exists.
4. **Click-anywhere-to-copy** (toasts, flash errors) stays, but gets the same
   visible "Copied ✓" confirmation and an explicit icon. Today nothing tells
   the user it exists or that it worked.
5. **Copy what the user can't see as well.** Truncated messages, stderr tails
   behind "Details", and session ids that only appear in a tooltip are all
   included.

### 4.5 Redaction

Error text can carry credentials: auth failures echo request details, and
stderr tails include whatever the CLI printed. `formatErrorReport` runs the
same shape-based redaction as the continuation packet
(`agentmux-srv/src/backend/continuity.rs`, `redact_secrets` /
`redact_private_keys`: GitHub, OpenAI/Anthropic `sk-`, Slack, AWS key
prefixes, and PEM private keys). It's ported to
`frontend/app/errors/redact.ts` with the same test vectors, so both stay in
step. Redaction runs on the whole report, before any truncation (the
redact-then-cap rule from #3673). The on-screen text is not changed, only
what's copied.

### 4.6 Pages without the frontend

- **CEF `data:` pages**: crash "AgentMux hit a problem"
  (`client/crash_recovery.rs:303-435`), crash loop and low memory
  (`client/recovery_pages.rs:26,155`), load error (`client/navigation.rs:1003`),
  install broken (`commands/window/creation.rs:169`).
  - Each gets a **Copy details** button next to its existing actions.
  - It's wired in a `<script>` with `addEventListener`, per the install
    page's test.
  - Transport is a hidden textarea + `execCommand`: an opaque-origin page has
    no secure context for `navigator.clipboard`.
  - The payload is the same §4.1 text, built in Rust. A shared
    `error_report_text()` in `agentmux-cef` mirrors `formatErrorReport`'s
    format, and includes the log paths from `get_log_paths`'s Rust side.
  - The install page's existing "Copy path" button is re-checked in the
    running app. It's guarded by `if (navigator.clipboard)` (:220), which is
    likely always false on a `data:` page, so it probably does nothing today.
- **Native dialogs** (Windows `MessageBoxW` at `lib.rs:1113`, launcher
  `show_fatal_dialog` at `main.rs:477`) are out of scope. Windows already
  copies a message box with Ctrl+C. Adding "Press Ctrl+C to copy this
  message" to their text is a cheap follow-up. Linux zenity/kdialog dialogs
  can't be copied at all; noted, not addressed.

## 5. Surfaces and where the button goes

**Kind:** E = error copy only; D = error copy plus Copy diagnostics and Reveal
logs. **P** = phase (§7).

| # | Surface | Where (file:line) | Kind | Placement | P |
|---|---|---|---|---|---|
| 1 | Agent failure row (auth, "No account linked", rate limit, crash) | `agent-view.tsx:2541-2558`, actions `failure/failure-accessory.ts:236-243` | E | Action before "Details"; copies title, meta line, detail and stderr tail. Text as srv's `AgentFailure::explain()` (`agents/failure.rs:~73`) | 1 |
| 2 | Built-in "Not signed in" row | `agent-view.tsx:1795-1812` | E | Same as 1 | 1 |
| 3 | Inline transcript `Error` / `HTTP N` rows ("[AgentMux] no credentials…") | `virtualization/DocumentRow.tsx:309-345` | E | Icon at the row's end, plus a context-menu entry | 1 |
| 4 | Connection-lost / "Can't reconnect" card | `app/init/error-display.ts:266` (actions :308-326) | D | "Copy details" next to Restore, `transport="dom"`; plus the §6.1 fixes so there's something to copy | 1 |
| 5 | Pre-launch "✗ Auth failed" | `components/PreLaunchAuthPanel.tsx:807-820` | E | Next to "Try again" | 1 |
| 6 | "Launch aborted" (picker) | `components/AgentPicker.tsx:1074-1084` | E | Inline icon | 1 |
| 7 | Host crash / hang page "AgentMux hit a problem" | `agentmux-cef/src/client/crash_recovery.rs:415-418` | D | In `.actions`, §4.6 | 2 |
| 8 | Crash loop, low memory | `agentmux-cef/src/client/recovery_pages.rs:26,155` | D | Next to their actions, §4.6 | 2 |
| 9 | Pane crash panel "This pane crashed" | `block/BlockErrorBoundary.tsx:214-233` | D | "Copy details" in the footer: name, message, stack, block id, view type, render trail (already assembled at :57). Keep copy on highlight | 2 |
| 10 | Generic `ErrorBoundary` fallback (workspace, tab, block, header) | `element/errorboundary.tsx:21-22` | D | Button overlaid on the `<pre>`, like `markdown-codeblock.tsx:65` | 2 |
| 11 | Backend "Offline" popover | `statusbar/BackendStatus.tsx:~205-245` | D | "Copy diagnostics" next to Restart Backend | 2 |
| 12 | Pane header render error icon | `block/blockframe.tsx:772-782` | E | Already copies on click; make that visible, not `disabled`-styled, and confirm it | 2 |
| 13 | Toasts and flash errors | `app/app.tsx:300-370`, `notification/notificationitem.tsx:67-69` | E | Keep click to copy; add the icon and "Copied ✓" | 3 |
| 14 | `ErrorBanner` (install flow) | `app/errors/ErrorBanner.tsx` | E | Next to the AMX code chip. Every future user of the banner gets it | 3 |
| 15 | Failed tool call output | `components/ToolBlock.tsx:401`, `ToolOverlayLog.tsx:55` | E | Icon in the overlay header, copies stdout + stderr + `[exited N]` | 3 |
| 16 | "Shell failed to start" | `components/AgentShellSubblock.tsx:738` | E | Inline icon | 3 |
| 17 | Session-outcome row ids | `virtualization/DocumentRow.tsx:617-676` | E | Icon copies attempted/actual session ids (tooltip-only today) | 3 |
| 18 | Armory and identity errors | `OAuthConnectPanel.tsx:261`, `identity-account-form.tsx:229,382`, `ClaudeLoginPanel.tsx:336`, `AgentIdentityPanel.tsx:168`, `AgentNewIdentityModal.tsx:124` | E | Replace with the exported `CopyableErrorMessage` | 3 |
| 19 | Modal `*-error` divs | `AgentLaunchModal.tsx:821`, `AgentStartupModal.tsx:85`, `mcp-manager.tsx:115`, `AgentMcpModal.tsx:75`, `AgentSkillsModal.tsx:79`, `bundle-manager.tsx:158,206`, `drone-view.tsx:908`, `CredentialApprovalWindow.tsx:102` | E | Same | 3 |
| 20 | Editor "Couldn't open file" and inline errors | `view/editor/editor-view.tsx:909-927` | E | Next to "Close tab"; the inline error gets the icon | 3 |
| 21 | Browser pane and load-error page | `view/browser/browser-view.tsx:148-150`; `agentmux-cef/src/client/navigation.rs:1003` | E | Icon; the CEF page per §4.6 | 3 |
| 22 | Media and Mermaid failures | `view/media/media.tsx:273,313`, `element/markdown-mermaid.tsx:88` | E | Icon | 3 |
| 23 | Config errors | `view/settings/settings-view.tsx:25-45`, `window/system-status.tsx:30-50` | E | Next to "Fix in editor" | 3 |
| 24 | Update / migration failed | `statusbar/MaintenanceSection.tsx:238,283-290`, `UpdateStatus.tsx:42` | D | "Update failed" first needs a detail to show (§6.3) | 3 |

Already have copy and need no change: the connection-status overlay
(`blockframe.tsx:943`), the install log's "Copy all" (`InstallProgress.tsx:86`),
the LSP install banner (`editor-view.tsx:955`), and `CopyableErrorMessage`'s
own two uses. These move to `<CopyErrorButton>` only if that makes their text
match §4.1.

## 6. Related fixes found while writing this

These make sure there's something worth copying:

1. **Startup errors lose their detail.** `app-init.ts:436-438` (and :553,
   :652, :748) log the error in a form that serializes to `{}`, and pass
   `String(error)`, which drops the stack.
   - Fix: one `describeError(e)` → `{name, message, stack, cause}`, used for
     both the log line and `showStartupError`.
   - The connection-lost card then shows, and copies, the real stack.
2. **The renderer can outrun srv at startup.** In the incident, srv started
   at 19:59:06.6 and was still running migrations when the renderer's first
   request failed at 19:59:08.086. `initHostMux` treated that as fatal and
   went straight to the connection-lost card.
   - Proposed: retry the initial srv request with a short backoff while srv
     reports "starting", before declaring the connection lost.
   - Separate PR. Root cause not confirmed beyond this timing.
3. **"Update failed" shows no reason** (`MaintenanceSection.tsx:238`,
   `UpdateStatus.tsx:42`). It needs the updater's error text before a copy
   button is worth adding.

## 7. Phases

| Phase | Scope | Surfaces |
|---|---|---|
| **P1** Foundations and the most-hit surfaces | `formatErrorReport` + `redact.ts` (with srv's test vectors), `<CopyErrorButton>`, exported `CopyableErrorMessage`, `describeError` (§6.1) | 1–6 |
| **P2** Crash class and diagnostics | `get_log_paths` IPC, Copy diagnostics, Reveal logs, the Rust `error_report_text()` for CEF pages | 7–12 |
| **P3** Everything else | Toasts, modals, Armory, panes, config, updates | 13–24 |
| **P4** Follow-ups | Startup retry (§6.2), update error detail (§6.3), native dialog hint (§4.6) | n/a |

P1 alone covers the errors users hit most, and it's one PR.

## 8. Verification

- **Real clipboard, not the check mark.** #2535 found copy buttons showing ✓
  while writing an empty string. Every phase's PR verifies at least its
  highest-traffic surface by reading the OS clipboard after a click (the
  method #2535 recorded), not only the UI state.
- **Unit tests**:
  - `formatErrorReport` per surface type: omitted lines, full stderr, stack
    kept intact.
  - `redact.ts` against the same vectors as `continuity.rs`'s tests,
    including a secret that straddles a cap.
  - `<CopyErrorButton>`: success, IPC failure falling back to `execCommand`,
    and total failure leaving the text selected with the Ctrl+C hint.
- **CEF pages**: Rust tests assert each page wires Copy with `addEventListener`
  (never an inline `onclick`) and embeds the report as a valid JS string
  literal, mirroring the install page's test (`creation.rs:1105-1118`).
- **Placement**: a lint-style test lists every component rendering an
  `*-error` class or an `ErrorBanner` / `ErrorBoundary` fallback, and fails if
  one has no `<CopyErrorButton>` or `CopyableErrorMessage`. That keeps new
  error surfaces from shipping without copy. The same check-script pattern is
  already used for other UI invariants in `scripts/`.

## 9. Open questions

1. Should Copy diagnostics offer to include the last N lines of the host and
   srv logs (redacted)? More useful to a developer, but more to review before
   sharing. Proposed: no for P2; revisit once reports show whether paths are
   enough.
2. Should the agent failure row's copy include the provider account's label
   (never the token)? It helps tell accounts apart in a report; it's also a
   personal email address. Proposed: the account's short id only.
