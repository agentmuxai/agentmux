# SPEC: Downloads in a browser pane open the system "Save as" dialog

**Status:** active — D1 (the download handler, the name sanitiser, the default folder, the per-pane limit) is built in the PR that adds `crates/cef/src/client/downloads.rs`. Not yet observed on a running build: the Save As dialog itself on Windows, macOS and Linux (D0 below). Not built: D2 (progress strip, Cancel, Show in folder), D3.
**Author:** Korp@narko, 2026-10-14, at the operator's request
**Trigger (operator, 2026-10-14):** "i downloaded it, but where do agentmux browser pane downloads go?" … "lets pivot to implement downloads … write a spec to file, clicking a download opens a system save menu where users can choose the path to save."
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §13 Q4 (downloads, left open) and §5.3 (the OS file dialog never opens for an agent-owned pane), `SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md` (the precedent for a CEF callback answered later and a prompt shown by the window that owns the pane).

## 1. Summary

1. **Today a download in a browser pane goes nowhere.** The pane's client has no download handler (§2), and the pane is an Alloy-style browser, for which CEF's default is to cancel the download. The operator clicked a download in a pane, saw nothing, and found no file in any normal folder.
2. **The fix:** a click on a download (a link with `download`, a response with `Content-Disposition: attachment`, a `blob:` link, a script-started download) opens the **operating system's own Save As dialog**, named after the file, in the folder last used (the OS Downloads folder the first time). Save writes the file; Cancel drops the download.
3. **How:** implement CEF's `DownloadHandler` for browser-pane clients and answer `on_before_download` with `cont(default_path, show_dialog = true)`. CEF shows the platform's own chooser (Windows common file dialog, macOS save panel, Linux GTK/portal), owned by the pane's window. We write no file code and no dialog code (§4).
4. **After saving**, the pane shows a small strip: file name, progress, **Cancel** while running, **Show in folder** when done (§5). It does not offer "Open": opening a file a web page chose is a separate decision (§7.3).
5. **Safe by construction:** the page never chooses where a file goes (§7.1), one pane can't flood the screen with dialogs (§7.2), and the main app window's behaviour does not change (§4.1).
6. **Phases:** D0 a spike that proves the dialog on all three platforms, D1 the dialog, D2 the progress strip, D3 agent-owned panes and the Save-link-as menu entry (§9).

## 2. What exists (verified on `main`, 2026-10-14)

| Piece | State |
|---|---|
| Client for a pane | `AgentMuxClient` in [`crates/cef/src/client/handlers.rs`](../../crates/cef/src/client/handlers.rs) implements display, keyboard, life-span, load, request, context-menu, drag, focus and permission handlers. It has `is_browser_pane`. **It has no `download_handler`.** No `DownloadHandler`, `on_before_download` or `can_download` exists anywhere in `crates/`. |
| Pane runtime style | Browser panes are created `RuntimeStyle::ALLOY` (`browser_pane/creation.rs`, `creation_views.rs`). For Alloy, CEF's header says a download with no handler is not allowed. Confirm against the pinned header in D0. |
| Host bridge command | `download_file` is listed among the stub commands (`commands/stubs.rs`) and does nothing. It is unrelated to a page's downloads and is not reused. |
| Native dialogs we already show | `rfd` (Windows common dialog, macOS panel, Linux GTK) in [`commands/platform.rs`](../../crates/cef/src/commands/platform.rs): the Media pane's open dialog and the layout Save/Open dialogs, each in `spawn_blocking`, wrapped in `host_dialog_guard()` so an "Always on top" floater can't hide the dialog (Windows). |
| Reveal a file in the OS | `platform.rs` already has the Explorer `/select,<path>` / `open -R` / `xdg-open` function behind the "open in file manager" command. |
| A CEF callback answered later | `media_prompt.rs`: callbacks are not `Send`, so they live in a UI-thread thread-local, resolved exactly once, cancelled when the pane closes. The same shape is needed here only if D0 shows that CEF's own dialog can't be used (§4.3). |
| Context menu | Browser-pane right-click is a DOM menu driven by the `browser-pane-context-menu` event; it receives `link_url`. It has no "Save link as". |
| `dirs` | `dirs::download_dir()` is already used by srv (`fs_ops`), so the crate is available. |

## 3. User experience

1. The user clicks a download in a pane. The pane's page does not change.
2. The OS Save As dialog opens, attached to the pane's window: **title** from the OS ("Save As"), **file name** the suggested one (§7.1), **folder** the last folder a pane download was saved in, otherwise the OS Downloads folder.
3. **Save:** the download starts. The strip at the bottom of the pane shows `report.pdf  4.2 MB of 18 MB  [Cancel]` (D2).
4. **Cancel in the dialog:** nothing is written, nothing is shown. (A page that starts a download by script and is cancelled doesn't get an error message to show.)
5. **Done:** the strip reads `report.pdf  saved  [Show in folder]` for a few seconds, then folds into the pane header's download list (D2) so it can still be reached.
6. **Failure** (disk full, network drop, permission): the strip says so in words, with the reason CEF gives, and offers nothing to retry (the page's link is still there).
7. **A second download while the first dialog is open** gets its own dialog, up to the limit in §7.2.

## 4. Design

### 4.1 Which clients get it

`AgentMuxClient::download_handler()` returns the handler **only for a real web pane: `is_browser_pane && !drag_capture`**. `drag_capture` is true for every client that renders the AgentMux frontend (main windows, tear-offs, floating panes), and a Windows floater is built as a browser-pane client for other reasons, so `is_browser_pane` alone is not enough. Those clients keep today's behaviour (no handler), so the app's own UI can never start a file save.

### 4.2 The handler

```
wrap_download_handler! {
    struct AgentMuxDownloadHandler { inner: Arc<Mutex<AgentMuxHandler>> }
    impl DownloadHandler {
        fn can_download(browser, url, request_method) -> c_int   // 1 = allow; §7.2 limit
        fn on_before_download(browser, item, suggested_name, callback) -> c_int
        fn on_download_updated(browser, item, callback)
    }
}
```

- `can_download`: allow, unless the pane is over its dialog limit (§7.2). Log `target: "pane-download"` with the pane id, URL **origin** (not the full URL, which can carry tokens) and method.
- `on_before_download`: build the default path (§4.4) and call `callback.cont(Some(&path), 1)` (show the dialog), then return `1` (handled). This is the same call CEF's own sample client makes. Record `item.id()` → pane in a small map so the later updates can be attributed (§5).
- `on_download_updated`: while the download runs, post throttled progress to the window that owns the pane (§5). On completion record the final folder as the new "last folder" (§4.4) and post `done`.

### 4.3 Why CEF's dialog first, `rfd` second

| | CEF dialog (`cont(path, true)`) | Our own `rfd` dialog, `cont(chosen_path, false)` |
|---|---|---|
| Code | A few lines; no thread, no parked callback | `spawn_blocking`, a UI-thread registry for the `BeforeDownloadCallback` (not `Send`), exactly-once resolution, pane-close cancellation: the whole `media_prompt.rs` shape |
| Owner window | The pane's own window, so it stacks with the right window and floater | Unowned, so it needs `host_dialog_guard()` and still can open on the wrong monitor/window after a tear-off |
| Control | Folder memory and filters come from CEF | Full: filters, titles, remembered folder |
| Risk | Dialog code lives in Chromium: behaviour must be checked on each OS | Same dialog library we already ship for the Media and layout dialogs |

**Decision:** D1 uses CEF's dialog. **D0 must prove it on Windows, macOS and Linux** (§8). If it fails on one (no dialog, wrong window, hangs with an "Always on top" floater, ignores the default folder), that platform falls back to `rfd` with the parked-callback shape from `media_prompt.rs`; the user-visible behaviour stays the one in §3.

### 4.4 Default path

`last_download_dir` (or `dirs::download_dir()` if none, or the home folder if neither exists) joined with the sanitised suggested name (§7.1). `last_download_dir` is the parent of the last *completed or started* path chosen in a pane dialog, kept in memory for the session in v1. Persisting it in settings is a follow-up. Don't create folders; if the remembered folder is gone, fall back to Downloads.

## 5. Progress, cancel, reveal

- **Events:** `pane-download-update` `{blockId, downloadId, name, state: "running" | "done" | "failed" | "cancelled", received, total (-1 if unknown), path (done only), reason (failed only)}`, sent to the window that **owns the pane** with `emit_event_to_window` and `browser_pane_window_label`, never to "main" and never into the page (the camera-access spec's rule: a page must not draw or answer its own prompts). Throttle to at most 4 events a second per download; `done`/`failed`/`cancelled` are never dropped.
- **Cancel:** the strip's button calls a new host command `browser_download_cancel {block_id, download_id}`. The host keeps the `DownloadItemCallback` in a UI-thread map (it is not `Send`, same reason as §4.3), removes it on any final state or when the pane closes, and calls `cancel()`. A cancel for an id the host no longer holds is a no-op.
- **Show in folder:** the strip's button calls the existing reveal-in-file-manager command with the path from the `done` event. The path comes from the host, never from the page or the strip's own text.
- **Pane close** while a download runs: cancel it (CEF cancels when the browser closes; the host drops its map entries and posts nothing, since the window no longer has the pane).
- **Strip** (D2): in the pane's frame component (`frontend/app/view/browser/`), driven only by the event. A text row, no icons: `name  progress  [Cancel]`. It is part of the pane chrome, outside the page's area, so a page can't imitate it.

## 6. Entry points

| Entry | Phase |
|---|---|
| Link with `download`, `Content-Disposition: attachment`, non-displayable MIME type, `blob:` and `data:` links, form POST returning a file | D1 (all go through the same CEF path) |
| Script-started download (`a.click()`, `location = …`) | D1, limited by §7.2 |
| Right-click **Save link as…** on a link, **Save image as…** on an image | D3: the DOM menu already receives `link_url`; add a host command that calls `browser.host().start_download(url)`, which re-enters this handler |
| `Ctrl+S` (save page) | Not in scope: Alloy does not route it, and a "save the whole page" format is a separate question |

## 7. Security

### 7.1 The page never chooses the path

The suggested name comes from the server's `Content-Disposition` or the URL, which is page-controlled text. The host:

- takes only the **file name** part (strips any directory and drive component, both `/` and `\`);
- removes control characters, `<>:"|?*` and trailing dots and spaces;
- avoids Windows reserved device names (`CON`, `NUL`, `COM1`…) by prefixing `_`;
- caps the length (200 characters, keeping the extension);
- falls back to `download` if nothing is left.

The dialog shows the result, and the user may change it. The host never writes to a path any page text decided: the only path used is the one the OS dialog returns.

### 7.2 A page can't flood the screen

A page can start downloads without a click. The dialog is the consent, so the limit is on how many may wait for one:

- **At most 3 downloads per pane waiting for a Save-As answer.** `can_download` returns 0 for a fourth, and the pane gets one line in its strip: "This page tried to start more downloads. Ignored." (D2; D1 just logs.)
- A download counts as waiting from `on_before_download` until its first `running`/`cancelled` update; the pane closing clears the count.
- The counter is a plain struct (pending-per-pane map plus limit) so it is unit-tested without CEF.

### 7.3 Opening, executing, marking

- **No "Open" button in v1.** Revealing the file in the file manager leaves the next step to the user and the OS, which is where its warnings live. An "Open" button on a downloaded executable is a one-click run of something a web page chose.
- **Mark of the Web / quarantine:** Chromium's download path marks files from the Internet (Windows `Zone.Identifier`, macOS quarantine attribute) so SmartScreen and Gatekeeper still apply. D0 verifies this on a saved file on each OS; if CEF's embedder path doesn't, that is a bug to fix before D1 ships, not a follow-up.
- **No access to the page's cookies from our side:** CEF performs the download with the pane's own session, as the page would. The host doesn't copy the URL or re-fetch it.

### 7.4 Privacy and logs

Log the pane id, the URL's origin, the sanitised file name's extension and the size, never the full URL (it can carry tokens) and never the full local path (it contains the user name). The event to the frontend carries the name and the path because the user sees them.

### 7.5 Agent-owned panes

An agent that opened a pane (`browser:owner_agent`, B1 of the parent spec) can click a download. The dialog would then appear on the human's screen for something the agent did, and block nothing the agent can answer (parent spec §5.3: the OS file dialog never opens for an agent). **D1 keeps the same behaviour for every pane** (the human answers; the agent's call has already returned; the strip shows who started it if the pane is agent-owned) so nothing silently saves. **D3** answers the parent spec's §13 Q4: for an agent-owned pane, save into the agent's workspace (`<workspace>/downloads/`, recorded by srv at spawn, not a path from block meta), no dialog, listed in the action log, with the size cap and the name rules above, and told to the agent as `saved <name> (<size>) to <path>`. That needs the operator's decision (§10 Q2) and a srv call to learn the workspace, so it is not in D1.

## 8. Tests

**Unit (no CEF process; `cargo test`)**
- Name sanitiser: `..\..\x.exe`, `a/b.txt`, `C:\x`, `CON.txt`, `name. `, 300-character names, only-illegal names, Unicode names, `.bashrc`, names with a double extension (`x.tar.gz`) keep their extension.
- Default-path builder: remembered folder missing → Downloads → home.
- Per-pane pending counter: the fourth is refused; a started or cancelled download frees a slot; a pane close clears; a double "started" for the same id doesn't free two.
- Event throttle: final states always pass, running updates ≤ 4/s.
- Event payload shape (golden JSON).

**Frontend (`vitest`)**: the strip's states (running, unknown total, done, failed, cancelled), Cancel and Show in folder call the right commands with the host-supplied ids/paths, an event for a pane that isn't this one is ignored.

**Manual matrix (D0 and again before D1 merges; Windows, macOS, Linux)** with a local test server that serves each case:
1. `Content-Disposition: attachment; filename="report.pdf"` link.
2. `<a download="x.txt" href="blob:…">` made by script.
3. A form POST that answers with an attachment.
4. A redirect that ends in an attachment.
5. `application/octet-stream` with no filename (the name comes from the URL).
6. A 500 MB file: progress, Cancel mid-way, no partial file left (or a clearly named one).
7. File names with spaces, Unicode, `..\` and a reserved name.
8. Save, then Cancel in the dialog, then Save again.
9. Close the pane while the dialog is open; close it while the download runs; tear the pane off into its own window with the dialog open.
10. A script that starts 10 downloads in a row.
11. The dialog with an "Always on top" floating pane on screen (Windows).
12. The saved file carries Mark of the Web / quarantine (§7.3).

**Acceptance:** from a signed-in portal that returns a one-time key file (the case that prompted this), the file lands where the user chose, exactly once.

## 9. Phases

| Phase | Ships | Unblocks |
|---|---|---|
| **D0** | A throwaway branch: add the handler with `cont(path, true)` and log lines; run the manual matrix rows 1, 2, 7, 8, 9, 11, 12 on each OS. Output: a short note in this spec saying what each OS did, and which platforms need the `rfd` fallback. Also confirm the Alloy default (§2) by logging today's behaviour with no handler. | the decision in §4.3 |
| **D1** | `download_handler` for pane clients; sanitiser; default path and last folder; the per-pane limit; logs; unit tests; the `rfd` fallback on any platform D0 failed | clicking a download saves a file where the user chose |
| **D2** | `pane-download-update`; the strip with Cancel and Show in folder; `browser_download_cancel`; the flood message | seeing, stopping and finding a download |
| **D3** | Agent-owned panes (§7.5), Save link/image as…, persisted last folder, optional "ask where to save" setting | agents and the right-click menu |

D1 is small enough for one PR; D0 is the only part that needs a hands-on OS pass.

## 10. Open questions

1. **Setting:** a switch "Ask where to save each file" (Chrome has it) would let a user turn the dialog off and save to Downloads. The operator asked for the dialog; default **on**, and the switch is D3 unless wanted sooner.
2. **Agent-owned panes (§7.5):** save to the workspace without asking, or keep asking the human? Proposal: workspace, no dialog, shown in the action log. Needs the operator's yes because it makes files appear without a human click.
3. **Download list:** a per-pane list (D2) is enough, or an app-wide Downloads view? Proposal: per-pane in v1; the Files pane already shows the Downloads folder.
4. **macOS and Linux dialogs through CEF:** Linux needs a file chooser backend (GTK, or the desktop portal inside the sandbox). D0 shows whether the packaged Linux build has one; if not, the `rfd` GTK dialog is the fallback.
5. **Resume after a network drop:** CEF can pause and resume. Not in v1: a failed download says why and the user clicks the link again.
