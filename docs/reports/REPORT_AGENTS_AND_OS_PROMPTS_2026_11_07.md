# REPORT: Agents and OS prompts: one layer for every system dialog, where an agent answering one is an escalated operation

**Date:** 2026-11-07
**Author:** Manoz@Area54, for task #1857 (the operator's question, relayed by AgentY, 2026-11-06)
**Status:** analysis, research only, no code. A spec and the build (#1858) follow once the operator decides.
**Checked against:** origin/main `cf71797c4` (v0.60.19). Web sources were read on 2026-11-07; a secondary page is marked as one.

> [!IMPORTANT]
> **No lab was run.** Section 6 is from vendor documentation and a code audit. Where it says "needs a lab", the operator's OK is needed first (through AgentY), because those runs use a real screen and the real mouse and keyboard. Decision 9 below asks for that OK.

The operator's words (2026-11-06): "that may need to be a new set of tooling that combines muxapi perhaps with computer use, have we explored that?", then "right, an Agent clicking an OS prompt is actually an escalated operation, but we still want agents to be able to do it".

## Decisions for the operator

| # | Decision | My recommendation |
|---|---|---|
| 1 | Shape of the feature | One gate in srv with two doors: `Browser` (tier 1) and a new `Desktop dialog` action group (tier 2). No new top-level tool. |
| 2 | Risk levels and their windows (§4.3) | Low 15 s, medium 30 s, high 30 s with the super agent's review (60 s when nobody reviewed), critical is not answered by an agent in v1. |
| 3 | "Always for this ..." (§4.5) | Low: always for an app, file dialogs only, inside a folder the person named. Medium: always per site and permission, browser panes only. High and critical: never. |
| 4 | Critical prompts (UAC, polkit and keychain passwords, admin sign-ins) | Not driven. The agent asks the person; the card says so. Nothing in this layer types or approves a password. |
| 5 | Which paths a file dialog may be given | Low only inside the agent's own workspace or a folder the person allowed; any other path is medium and shows the whole path. |
| 6 | The agent's reason | Required, shown on the card as the agent's own words in quotes, never as part of the banner's wording. |
| 7 | The Desktop blocklist's "security and system prompts" entry | Stays. This layer is the one narrow door, with its own classifier and levels; it is not a way to un-block those windows for `Desktop click`. |
| 8 | Stage order | Tier 1 first (no OS API risk, all three platforms), then Windows tier 2, then macOS, then Linux. |
| 9 | A lab for tier-2 reliability (§6, §8) | Your OK to run one short, scripted check per OS (Notepad Save As on Windows, TextEdit on macOS, a text editor on Linux), with someone at the machine. |

## 1. The short answer

Yes, it has been half explored, and it fits what is already built. Three findings:

1. **Most of the dialogs an agent meets can be avoided, not answered.** In a browser pane AgentMux owns the process, so it can answer in code (tier 1). For a native app, the computer-use skills ladder already puts the app's own API ahead of clicks, and an API save never opens a dialog.
2. **The escalation machinery already exists.** `crates/srv/src/backend/sensitive_ops.rs` holds one table of operations, each with a countdown, a veto, a banner built from structured facts, and an optional review by the super agent (`SPEC_SUPER_AGENT_2026_10_25.md`). The audit log and the `Sensitive` routes are there. Answering an OS prompt is one more row in that table, not a new mechanism.
3. **What is missing is the reading.** Desktop today (Windows only) takes pixel clicks. It has no UI Automation tree, no password-field guard and no knowledge of dialogs. Tier 2 needs a structural reader on each OS, which is Phase 2 and later of `SPEC_DESKTOP_DRIVING_2026_10_20.md`, plus a classifier that says what a window is a prompt *for*.

## 2. Inventory: every prompt an agent can meet

"Owner" is the process that draws the prompt. It decides the tier.

| Prompt | Owner | Tier | Level | Built today |
|---|---|---|---|---|
| Page `alert`, `confirm`, `prompt`, `beforeunload` in a browser pane | AgentMux (CEF) | 1 | low | Built (D-1): parked, asked in the pane, 11 min cap, flood brake |
| Browser file download, Save As | AgentMux (CEF) | 1 | low | Dialog shown to the person; agent-owned panes saving to the workspace with no dialog is D3, not built |
| Browser file chooser (`<input type=file>`) | AgentMux (CEF) | 1 | low | An agent uploads with `Browser` `set_files`, and clicking a file input is refused with that hint. No `CefDialogHandler`, so a chooser the page opens itself shows the OS dialog to the person and can sit there |
| Browser permission (camera, mic, location, notifications) | AgentMux (CEF) | 1 | medium | The prompt path exists, but the grant store is empty, so every request is denied (Phase 2c unbuilt) |
| Browser HTTP auth | AgentMux (CEF) | 1 | medium | Parked and asked of the person, 5 min cap; the agent is not told when it expires |
| Browser certificate error, external protocol (`mailto:`) | AgentMux (CEF) | 1 | medium | Not handled; out of scope of the native-dialogs spec |
| Common file dialog (Open, Save, Save As, Browse) in a native app | The app (Windows Common Item Dialog, macOS panel, GTK/Qt or portal) | 2 | low (medium outside the allowed folders) | No special handling on any OS |
| App's own "Are you sure?", "Replace file?", "Save changes?" | The app | 2 | low | None |
| macOS TCC (Accessibility, Screen Recording, Files and Folders, Camera, Microphone) | The OS (`tccd`, System UI) | 3 | critical | Not drivable; the person grants |
| macOS Gatekeeper first-open ("downloaded from the internet", "cannot be opened") | The OS | 2 for the notarized "Open" prompt, 3 for "Open Anyway" in System Settings (my reading of how the prompts work; needs a lab) | high | None |
| Windows SmartScreen ("Windows protected your PC") | The OS, in the user's session | 2 | high | None. Probably refused by `Desktop` today under its "security and system prompts" blocklist entry |
| Windows UAC consent | The OS, on the secure desktop | 3 | critical | The OS refuses input into it; `Desktop` says so |
| Linux polkit authentication, keyring unlock | The desktop's agent | 3 (it asks for a password) | critical | None |
| Linux portal prompts (FileChooser, OpenURI, screencast) | `xdg-desktop-portal` backend | 2 for the FileChooser, 3 for screencast and the rest | low for FileChooser | None |

## 3. The tiers

**Tier 0: do not meet the dialog.** The computer-use skills ladder (`api`, `mcp`, `shortcuts`, `desktop`) already prefers the app's own route. A Word or Excel save through COM, a Blender render through `bpy`, a command-line export: no dialog. A skill's manifest should say where it can still meet one, so the agent reaches for tier 1 or 2 knowingly.

**Tier 1: MuxAPI answers in code, no dialog appears.** Only where AgentMux owns the process: the CEF browser pane. CEF has a handler for each prompt: `CefDialogHandler::OnFileDialog` (file chooser), `CefJSDialogHandler` (built), `CefPermissionHandler::OnShowPermissionPrompt`, and the request handler's auth callback. What is missing is registering the first, and creating grants for the third. This tier is the same on all platforms and has no OS reliability risk.

**Tier 2: Desktop fills the standard dialog structurally.** For a native app's own file dialog and its plain confirmation prompts. It reads the prompt through the OS accessibility API, classifies it, then sets the path field and presses the named button. It uses no pixel clicks: a pixel click on a dialog is the thing `SPEC_DESKTOP_DRIVING` already warns about (stale coordinates, a covering window).

- Windows: UI Automation on the Common Item Dialog (the File name edit and the Save or Open button).
- macOS: Accessibility on the open and save panel, with the Go to the folder sheet as the path entry.
- Linux: AT-SPI on the GTK or Qt dialog, or on the portal backend's FileChooser window.

**Tier 3: what cannot be driven, and what the person gets.** The secure desktop (UAC), TCC, polkit and password prompts. The OS built these to refuse automation. The agent is told the prompt needs the person (the same `pending` answer `Desktop` uses, and an `[AgentMux]` note when the person has acted), and the card in its pane names the prompt so the person knows what to look for. The agent never waits on it: the call answers at once.

## 4. The escalation model

### 4.1 One row in an existing table

Add `os_prompt_answer` to `OPS` in `sensitive_ops.rs`, with the flow `Sensitive` already runs: request, optional super-agent review, countdown, veto or approve-now, audit. This is the operator's rule applied as written: the agent is allowed to do it, the person can stop it in time, nothing is "operator only", and every step is recorded as the agent that did it.

### 4.2 What the card says, and who wrote it

The banner's words are built from structured facts (`OpSpec::check_facts`), never from the requester's own words. That rule stays. Each fact has a source:

| Fact | Written by | Example |
|---|---|---|
| `agent` | srv (the caller's identity) | Posa@Area54 |
| `app`, `window` | AgentMux, read from the window | Notepad, "Save As" |
| `prompt` | AgentMux, read from the dialog (its title and message text) | "Replace file? report.docx already exists" |
| `answer` | The request, checked against what AgentMux read: it must be a button the dialog has, or a path | "Save to C:\\Users\\area54\\work\\report.docx, press Save" |
| `level` | srv's classifier (§4.3) | low |
| `reason` | The agent, required, at most 200 characters | "the person asked me to save the report to my workspace" |

A request that names a button or a prompt the dialog does not have is refused, so an agent cannot ask the person to approve one thing and press another. The reason is shown in quotes under "The agent says:", so the one agent-written line is clearly the agent's.

### 4.3 Levels

| Level | What it is | Window | Super-agent review |
|---|---|---|---|
| Low | A file dialog in an app that already has a `Desktop` grant, with a path in the agent's workspace or a folder the person allowed; a page's own `alert` or `confirm`; a Save or Replace confirmation of that same dialog | 15 s (the stop and close precedent) | No: the window is the oversight |
| Medium | A browser permission (camera, mic, location, notifications, clipboard); HTTP auth; a file path outside the allowed folders; external protocol launch | 30 s | Yes |
| High | SmartScreen "Run anyway", Gatekeeper "Open", an installer's trust prompt, anything that lowers an OS protection for one file | 30 s, 60 s when nobody reviewed | Yes |
| Critical | UAC, polkit and keychain passwords, admin sign-in, a new TCC permission | None: tier 3 | n/a |

The windows follow the operator's rule (30 s by default, at most 2 minutes, and only for the critical operations). A 15 s low window is the one place I propose going under the default, because the existing stop and close windows are already 15 s and a file dialog is the commonest case.

### 4.4 The card and the countdown

The card is the `SensitiveBanner` the other operations use, in the agent's pane (and an OS toast, as for stop and close): the agent, the app, the prompt, the exact answer, the reason, the countdown, **Don't answer** and **Answer now**. The call never waits for it: with no standing permission, srv opens the card and answers the tool call `pending` at once, and the result arrives as an `[AgentMux]` note. This is the same rule as `Desktop`'s first grant (§5.4 there), so an agent stays free to take the person's messages. Launcher proof gates the buttons on the toast, as for the other operations.

### 4.5 "Always"

| Level | May be remembered? | What exactly |
|---|---|---|
| Low | Yes | For one app and one prompt kind (file dialogs), and a folder prefix. Not "any path". It is a setting, listed and revocable in Settings next to the browser grants. Each answer is still audited and still shows the Driving strip with Stop. |
| Medium | Yes, browser panes only | Per site and permission, as the Vault's "Always on this site" does. |
| High | No | Every time. |
| Critical | No | Not offered. |

An "always" is a standing grant, so creating one is itself a row in the same table (the existing `vault_always_allow` is the pattern).

### 4.6 Audit

Add to the audit entry (`rpc_types/audit.rs`): `prompt` (the dialog's text as read), `answer` (the exact value given), `level`, and `standing` (true when a remembered grant allowed it). Today's schema carries a summary, an outcome and a free `reason`, but not these. Typed text is never recorded, only a path or a button label, and never a password or a secret field (§5 of the Desktop spec: the guard refuses a password field).

### 4.7 What stays as it is

- **The Desktop blocklist.** Its "security and system prompts" entry stays. This layer reads those windows only to classify and answer them through the gate; it never lets `Desktop click` act on them.
- **Per-app Desktop grants.** A tier-2 answer needs the app to be granted already. A prompt from an app that is not granted is classified and refused with the reason.
- **Secrets.** The layer does not type into a password field and does not answer a credential prompt. A Vault entry stays the way a secret reaches a field, with its own approval.
- **Forging the person.** Nothing here records the person's words or approval except their own button or veto, written as theirs.

## 5. What is built today, and the gaps

| Area | Built | Gap |
|---|---|---|
| Browser pane JS dialogs | Yes (D-1): parked, asked, flood brake, 11 min cap | No agent action to cancel a `confirm` or answer a `prompt` (D-3) |
| Browser downloads | D1 and D2: Save As is shown to the person, progress strip, cancel | Agent-owned panes saving to the workspace with no dialog (D3, §7.5): needs the operator's yes |
| Browser file chooser | Upload by `set_files`; a click on a file input is refused with that hint | No `OnFileDialog` handler, so a page-initiated chooser shows the OS dialog |
| Browser permissions | Prompt path and per-pane grant store exist | The store is never filled: all requests are denied. Phase 2c |
| Browser HTTP auth | Parked, 5 min | The agent is not told when it expires |
| Certificate errors, external protocols | No | Deny and report by default |
| `Desktop` on Windows | `apps`, `look`, `click`, `scroll`, `type`, `key`, `wait`, `release`; per-app grants, blocklist, limits, Stop, audit | No UI Automation tree, no password-field guard, no dialog knowledge |
| `Desktop` on macOS and Linux | No: it answers "Windows only for now" | Phases 3 and 4 of its spec |
| Escalation card, countdown, audit | Yes, for 14 operations in `sensitive_ops.rs` | No row for this; the audit has no prompt, answer or level |

## 6. Reliability of tier 2, by OS

From documentation. Confidence is mine; "needs a lab" means a real run on a real screen.

| OS | Mechanism | What the sources say | Confidence | Risks |
|---|---|---|---|---|
| Windows | UI Automation on the Common Item Dialog (`IFileDialog`) | The dialog is in the app's own process, and Microsoft's own guidance is to use UI Automation rather than window handles. `IFileDialog` has a File name edit and a default button. | High that it is reachable; needs a lab for the exact control names on Windows 10 and 11 | Apps with a custom dialog (older `GetOpenFileName`, Qt, Electron's own); a dialog in an elevated app (refused); per-monitor DPI does not matter, since no pixels are used |
| macOS | Accessibility on the panel; Cmd+Shift+G ("Go to the folder") to enter a path | Since macOS 10.15 Open and Save panels are always drawn in a separate process (the powerbox), whether or not the app is sandboxed (Apple's documentation). The panel is presented as part of the app's window, so it should appear in the app's accessibility tree, but I found no source that says so. | Medium: needs a lab | Needs the Accessibility permission for AgentMux (itself a TCC prompt the person grants, tier 3); the remote panel's tree may be partial; Cmd+Shift+G needs synthetic keys |
| macOS | TCC prompts | Camera, Microphone, Screen Recording and Input Monitoring cannot be pre-approved even by an MDM profile; Accessibility can be. | High | Not drivable; do not try |
| Linux | AT-SPI on the dialog; the portal's FileChooser backend draws the dialog | `OpenFile` and `SaveFile` end only when the user's interaction ends (the response signal), and a backend draws the UI. AT-SPI actions (`doAction`, set text) are not input injection, so unlike `XTest` they may work on Wayland too. | Low to medium: needs a lab | GTK4 and Qt's accessibility coverage of the file chooser's path entry; per-desktop backends differ (GNOME, KDE, wlroots) |
| Windows | UAC consent | The prompt is protected from cross-process input. UI Automation reaches protected UI only with `uiAccess="true"`, a signed program in a trusted folder, and Microsoft says such a program must be highly trusted. | High | I recommend against building it: a signed `uiAccess` helper that clicks UAC defeats the point of UAC |
| Windows | SmartScreen | Triggered by the Mark of the Web on a downloaded file (`Zone.Identifier`). A person can clear it with `Unblock-File`. | Medium: the prompt is in the user's session, so UI Automation can see it | Answering it is high; clearing the mark by script is the same decision in another form and gets the same level |
| macOS | Gatekeeper | Checks the `com.apple.quarantine` attribute set by the downloader. `xattr -d` removes it; `spctl --master-disable` needs root (secondary sources for the commands). | Medium | Same: removing the attribute is a high operation, not a shortcut around the card |

**A lab I would want, once the operator says yes** (section 9, decision 9): one short scripted check per OS of the file dialog only: Windows Notepad Save As, macOS TextEdit Save, a Linux text editor Save As. Each reads the dialog, sets a path, presses Save, and verifies the file exists. Someone at the machine, no loop, one or two runs.

## 7. A staged plan

| Stage | Content | Done when |
|---|---|---|
| 0 | This report; the operator's decisions | decided |
| 1 | The gate: the `os_prompt_answer` row, the audit fields, the card with the reason, a `prompt` and `level` classifier for tier 1. Browser: register `OnFileDialog` (cancel and report, so a page-initiated chooser cannot hang), fill the media grant store through the gate, deny and report certificate errors and external protocols, tell the agent when an auth prompt expires, D3 (agent-owned panes save to the workspace, if the operator agrees) | An agent saves a download and uploads a file in a browser pane with no dialog, and a camera prompt shows the card |
| 2 | Windows tier 2: UI Automation tree and password-field guard in `Desktop` (Phase 2 of its spec), then `Desktop dialog` (`look`: the classified prompt; `answer`) for the Common Item Dialog and plain confirmations | Notepad Save As answered through the card, and refused for an app that has no grant |
| 3 | macOS tier 2: the Accessibility permission flow, the panel through AX | TextEdit Save answered |
| 4 | Linux tier 2: AT-SPI, the portal FileChooser | A text editor's Save As answered |
| 5 | High-level prompts: SmartScreen and Gatekeeper classified and answered at level high | One of each, on a downloaded installer the person asked for |
| not planned | Critical (UAC, polkit, passwords, TCC) | Stays with the person |

Stage 1 stands on its own and does not wait for any OS API. Each of 2 to 4 ships behind `desktop:enabled`, which is off by default.

## 8. Risks

- **A card that can be misled.** The prompt text and the buttons are read by AgentMux, not supplied by the agent, and an answer that does not match what was read is refused. The remaining agent-written text is the reason, labelled as the agent's.
- **Reading the wrong window.** A dialog from another app can stack over the target. As in `Desktop`, the answer is checked against the window that owns the dialog and refused when another window is in front.
- **Path exposure.** An Open dialog can read any file into an app that uploads it. The path policy (decision 5) and the full path on the card are the guard.
- **"Always" creep.** A remembered grant is a standing permission. It is a setting, listed, revocable, and audited on every use; high and critical never get one.
- **Wayland.** Not a problem for tier 1. For tier 2 it depends on AT-SPI actions, which no source I read confirms for the file chooser; the lab decides.

## 9. Which decisions matter most

All nine are in the table at the top, each with my recommendation. The ones that change what an agent can do without asking are 3 (always), 5 (paths) and 7 (the blocklist). Decision 9 is the only one that needs the operator before any work, because it is a run on a real screen.

## 10. Sources

- Microsoft, [Common Item Dialog](https://learn.microsoft.com/windows/win32/shell/common-file-dialog) and [IFileDialog](https://msdn.microsoft.com/en-us/library/bb775966(v=vs.90)).
- Microsoft, [UI Automation Security Overview](https://msdn.microsoft.com/en-us/library/dd319580(v=vs.85)) (protected UI, `uiAccess`, signing).
- Apple, [NSOpenPanel](https://developer.apple.com/documentation/appkit/nsopenpanel.md) and [NSSavePanel](https://developer.apple.com/documentation/appkit/nssavepanel.md) (panels in a separate process on macOS 10.15 and later); [the NSRemoteView thread](https://developer.apple.com/forums/thread/72133) (the powerbox).
- Flatpak, [FileChooser portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html) and [its backend interface](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.FileChooser.html).
- Jamf and Hexnode on PPPC: [Hexnode, TCC and PPPC](https://www.hexnode.com/forums/topic/what-exactly-are-tcc-and-pppc-on-macos-and-how-do-it-admins-use-them-in-managed-environments/) and [Iru, screen recording](https://docs.iru.com/en/endpoint/library/library-items-profiles/configure-screen-recording-permissions) (secondary).
- CEF, [`cef_dialog_handler.h`](https://third-party-mirror.googlesource.com/cef/+/refs/heads/master/src/include/cef_dialog_handler.h) (a mirror; `OnFileDialog`), and the permission handler's `OnShowPermissionPrompt`.
- Mark of the Web and SmartScreen: [Eric Lawrence, Downloads and the Mark of the Web](https://textslashplain.com/2016/04/04/downloads-and-the-mark-of-the-web) and [winhelponline](https://www.winhelponline.com/blog/bulk-unblock-files-downloaded-internet/) (secondary).
- Gatekeeper's quarantine attribute: [MITRE T1553.001](https://attack.mitre.org/techniques/T1553%2F001/) (the `xattr` and `spctl` commands are from secondary pages).
- `pkexec(1)`, the manual page (the authentication agent and its dialog).
- In this repo: `docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md`, `SPEC_BROWSER_PANE_NATIVE_DIALOGS_2026_10_17.md`, `SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md` §7.5, `SPEC_SUPER_AGENT_2026_10_25.md`, `crates/srv/src/backend/sensitive_ops.rs`, `crates/cef/src/client/jsdialog.rs`, `crates/cef/src/client/downloads.rs`, `crates/cef/src/browser_panes/media_grants.rs`, `crates/mcp/src/tools/desktop.rs`.
