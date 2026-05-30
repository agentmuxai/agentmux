# SPEC: Drag-and-drop files into Terminal and Agent panes

**Date:** 2026-05-30
**Status:** Draft
**Owner:** Frontend + CEF host + sidecar
**Scope:** drag-and-drop of files from the OS file manager (and inter-pane in v2) onto a Terminal or Agent pane, with both **host-local** and **containerised/remote** agents supported.

---

## 0. Why this is non-trivial

Two facts make file DnD unusual in AgentMux:

1. **The CEF browser sandbox hides full filesystem paths.** A drop event on a CEF/WebView2 pane exposes only `File` objects with bare filenames (e.g. `report.csv`). The host path (e.g. `C:\Users\me\Downloads\report.csv`) is *not* in the JS event — it has to be captured Rust-side by a `CefDragHandler` and surfaced through IPC. The current placeholder in `frontend/app/view/term/term.tsx:330–344` documents the gap explicitly.
2. **The terminal's working directory may not live on the host.** For container/SSH/WSL-backed connections, "copy this file into the CWD" means cross-boundary file transport — not a `std::fs::copy`. The current `copy_file_to_dir` IPC (`agentmux-cef/src/commands/providers.rs:685`) is host-local-FS only.

A spec that ignores either point produces a UI that "works" on a developer laptop and silently breaks the moment a real user drops a CSV onto a Dockerised agent.

---

## 1. Current state (audit)

Already merged / in repo:

| Surface | Status |
|---|---|
| `DragOverlay` UI in `term.tsx` (the "Copy to /path" hint shown on drag-enter) | ✅ shipped |
| `dragover` / `drop` listeners gated on `detectHost() === "cef"` | ✅ shipped |
| `copy_file_to_dir` IPC | ✅ shipped, local-FS only |
| `cmd:cwd` meta seeded on shell spawn (`shell.rs:690+`) so the destination is known | ✅ shipped |
| `handleFilesDropped` calling `copy_file_to_dir` per file | ✅ shipped |

Not yet implemented (the part this spec specifies):

| Gap | Symptom today |
|---|---|
| CefDragHandler → IPC bridge to get host paths | Toast: "Full path copy requires CefDragHandler integration." Drop is a no-op. |
| Container / SSH / WSL transport for `copy_file_to_dir` | Would silently copy to a host path that doesn't exist inside the container |
| Agent-pane drop handler | Drop on an Agent pane is unhandled — falls through to the browser default (navigates away on dev, no-op in CEF) |
| Multi-file batch + progress UX | Each file fires an independent toast; large files block UI |
| Inter-pane file drop (drag from File pane → Terminal pane) | Not in scope here — separate spec |

---

## 2. Best-practice research

What other terminals and AI-IDE chat surfaces do on file drop:

| Product | Behavior | Why |
|---|---|---|
| **VS Code (integrated terminal)** | Pastes the file's quoted host path at the cursor | The terminal user is presumed to be operating *on* the host; the path *is* the deliverable |
| **iTerm2** | Pastes `\`-escaped path; modifier (⌥) triggers `scp` upload to remote-session CWD | Recognises the "remote session needs the bytes, not the string" case |
| **Windows Terminal / Hyper** | Pastes the path | Same model as VS Code |
| **macOS Terminal.app** | Pastes the path | Same |
| **Cursor / Claude Code chat input** | Attaches file to context as a chip; agent reads it on next turn | The file is *information for the AI*, not a deliverable to a shell |
| **ChatGPT desktop** | Same as Cursor: attaches to message |
| **Cline / Continue / Aider** | Adds file to the agent's tracked-context list ("/add path") |
| **Warp** | Smart: detects shell, infers `cp` / `scp` / `cat` depending on what's selected |

**Two distinct mental models emerge:**

- **Terminal model**: "the path is the payload." Drop = paste path. Works because the user is at a shell that already has access to that path. Fails when the shell is on a different machine or inside a container.
- **AI/chat model**: "the bytes are the payload." Drop = attach as context. Path is irrelevant; the agent gets the content.

AgentMux's user direction ("copy it to the working directory") chooses neither model — it picks a third:

- **Materialise-into-CWD model**: "the bytes are the payload, AND the agent reads from disk by path." Drop = transport the file into the agent's working directory so the agent can `read_file CWD/report.csv` in its very next tool call.

This is the right choice for AgentMux specifically because:
- The agent's main read path is filesystem tool calls, not "attach to chat".
- The CWD is where the agent already looks for project files. Putting a file there makes it discoverable without a separate "I attached X — please read it" turn.
- It works identically for host agents and container agents (provided the transport is right) — the agent's prompt and tool calls don't change.

**Best practice cited:** Cursor's `@file` attach + JetBrains AI Assistant's drop-into-context both ALSO materialise to disk under the hood for tools that need a path. This isn't a new model — it's the contract Cursor uses too.

---

## 3. Design

### 3.1 Behavior matrix

| Pane type | Connection | Drop result |
|---|---|---|
| Terminal | `local` (host) | Copy file → `cmd:cwd` on host. Toast on success. |
| Terminal | container / SSH / WSL | Stream file → `cmd:cwd` inside the remote. Toast on success. |
| Agent | `local` | Copy file → `cmd:cwd` on host **and** insert a reference token (`@filename`) into the composer at the caret. |
| Agent | container / SSH / WSL | Stream file → `cmd:cwd` inside the remote **and** insert `@filename` into the composer. |

Why a reference token in the agent case: the agent doesn't know to read the new file unless we tell it. The token is a signal in the next user turn ("@report.csv") that the agent's prompt template can expand to "the user dropped report.csv into the CWD; consider it new context." This matches Cursor / Continue behavior.

### 3.2 The destination is always `cmd:cwd`

`cmd:cwd` is already broadcast to the frontend on shell spawn (`shell.rs:690+`) and is the single source of truth for the active working directory. No new meta key. If `cmd:cwd` is missing, drop is rejected with the existing "No working directory detected" toast — the same UX the current half-implementation has.

### 3.3 Capturing the host path (CEF)

The HTML5 path-hiding limitation is solved by a `CefDragHandler` on the Rust side. The flow:

```
OS drag enters CEF view
  → CefDragHandler::OnDragEnter (Rust)
    → reads CefDragData::GetFileNames() — these ARE full paths
    → stashes the path list keyed by pane window id
  → JS drop event fires
    → JS calls invokeCommand("consume_drag_paths", { windowId })
    → Rust returns the stashed list and clears it
    → JS calls handleFilesDropped(paths) as it does today
```

Critical detail: `CefDragHandler::OnDragEnter` must **not** call `SetDragData(null)` or return `true` to swallow — that suppresses the JS event entirely. We let the JS event fire so the existing UI flow (overlay show/hide, per-file iteration) keeps working unchanged; CEF just supplies the missing piece (paths).

A 5-second TTL on the stash prevents leaks if the JS event doesn't fire (window unfocused mid-drag, drop into a non-target region). Cleanup also fires on the next OnDragEnter.

**Linux/macOS:** CEF's `OnDragEnter` behaves the same on all three platforms — the same handler covers all three. (Tauri/WebView2 do not; we are CEF-only now per `agentmux/CLAUDE.md`, so the simpler path holds.)

### 3.4 Cross-boundary transport (`copy_file_to_dir_v2`)

Rename / extend the existing IPC into a connection-aware command that dispatches on `meta.connection`:

```rust
pub fn copy_file_to_dir_v2(args: &Value) -> Result<Value, String> {
    let source = args["source_path"].as_str().ok_or(...)?;
    let target_dir = args["target_dir"].as_str().ok_or(...)?;
    let conn_name = args["connection"].as_str().unwrap_or("local");

    match resolve_connection(conn_name) {
        Conn::Local           => copy_local(source, target_dir),
        Conn::Ssh(cfg)        => copy_via_sftp(source, target_dir, cfg),
        Conn::Container(cfg)  => copy_via_docker(source, target_dir, cfg),  // `docker cp`
        Conn::Wsl(distro)     => copy_via_wsl(source, target_dir, distro),  // `wsl -d <distro> -- cp`
    }
}
```

The frontend always passes the source pane's `meta.connection` along with the source path. The sidecar (not the CEF host) actually owns the transport implementations — the CEF host just forwards to a new sidecar RPC `FileTransportCommand`, because (a) sidecar already owns connection configs and (b) the host should not link an SSH/Docker SDK.

The local-FS path stays exactly as it is today (`std::fs::copy` + `deconflict_path`). The Local transport variant is a thin shim over the existing function — zero regression risk for the already-shipped host-only case.

### 3.5 Filename de-collision

Reuse the existing `deconflict_path` helper. If `report.csv` already exists in the CWD, the new file lands as `report (1).csv`, then `report (2).csv`, etc. The returned destination is what's used in the toast and (for Agent panes) in the composer token.

### 3.6 Agent-pane composer integration

After a successful transport, the composer textarea receives an insertion at the current caret position:

```
" @report (1).csv"      // leading space if caret isn't at start, trailing nothing
```

Implementation lives in a new hook `useAgentDropAttach` consumed by the agent view. It:

1. Calls the same DragOverlay UI shown on terminal panes (consistent visual language).
2. Calls the same transport (3.4) — same code path, same toast, same de-collision.
3. On success, walks up to the composer textarea via the agent view's existing `composerRef` and splices the `@filename` token at the caret.

If the composer isn't mounted (rare race during pane init), the token is queued in agent-view state and inserted on the next composer mount.

### 3.7 Multi-file UX

Drop of N files:
- A single overlay shows "Copy 3 files to /path" (count, not list).
- Transport runs concurrently with a semaphore of 4 (avoids OOM on big drops).
- A single toast on completion summarises: "Copied 3 files to /path." Failures are appended ("2 copied, 1 failed: too large").
- For Agent panes, all `@filename` tokens are concatenated and inserted as one splice (so the user can backspace once to remove the whole attach list).

### 3.8 Large files

Hard cap: **256 MiB per file** by default, tunable via `dnd:maxfilesizemb` setting. Above the cap the drop is rejected with a toast — we do not want a 4 GiB ISO to silently pin the sidecar for 20 minutes. The cap is enforced sidecar-side (after the file is staged, before transport begins) so the host can't be tricked into starting a transfer it'll abort.

### 3.9 Progress

For files > 8 MiB a progress toast replaces the success toast: a progress bar that updates from the transport's byte counter (already wired for sftp / docker cp via the underlying crates). At < 8 MiB we don't bother — the transport is faster than the toast animation.

---

## 4. Implementation surface

### Frontend (TypeScript)

| File | Change |
|---|---|
| `frontend/app/view/term/term.tsx` | Replace the placeholder branch in `onDrop` with a call to `invokeCommand("consume_drag_paths", ...)` then `handleFilesDropped(paths)`. Keep the existing CWD-missing toast. |
| `frontend/app/view/agent/components/AgentDropZone.tsx` (new, ~80 LoC) | Mirrors the term DragOverlay flow. Owns the composer-token splice. |
| `frontend/app/view/agent/hooks/useAgentDropAttach.ts` (new, ~120 LoC) | Wires DragOverlay + drop event + composer splice + queue-on-unmount. |
| `frontend/util/dnd.ts` (new, ~60 LoC) | Shared `consume_drag_paths` invoke + multi-file iteration + de-collision result handling. Used by both term and agent. |

### CEF host (Rust)

| File | Change |
|---|---|
| `agentmux-cef/src/client/handlers.rs` | Implement `CefDragHandler` — stash `GetFileNames()` keyed by window id, 5 s TTL. |
| `agentmux-cef/src/ipc.rs` | Register `consume_drag_paths` (returns + clears the stash). |
| `agentmux-cef/src/commands/providers.rs` | `copy_file_to_dir` → `copy_file_to_dir_v2`. v1 stays as a deprecated alias for one release (back-compat for any out-of-tree consumer; nothing in-tree calls it). |
| `agentmux-cef/src/commands/file_transport.rs` (new) | Thin dispatcher to sidecar RPC for non-local connections; passes through to local copy for `"local"`. |

### Sidecar (Rust)

| File | Change |
|---|---|
| `agentmux-srv/src/backend/file_transport/` (new module) | `local.rs`, `ssh.rs` (sftp via the existing ssh crate), `docker.rs` (`docker cp` via process spawn — no SDK), `wsl.rs` (`wsl -d <distro> --` shell). Each implements a small `Transport` trait. |
| `agentmux-srv/src/backend/rpc_types.rs` | New `FileTransportCommand { source_path, target_dir, connection, max_bytes }` returning `{ dest_path, bytes_transferred }`. |
| `agentmux-srv/src/backend/rpc.rs` (handler dispatch) | Wire the command to a `file_transport::dispatch()`. |

### Settings

| Key | Default | Description |
|---|---|---|
| `dnd:enabled` | `true` | Master kill-switch — disables both term and agent drop |
| `dnd:maxfilesizemb` | `256` | Per-file cap (3.8) |
| `dnd:concurrency` | `4` | Semaphore size for multi-file (3.7) |
| `dnd:agentinserttoken` | `true` | Whether agent panes also splice an `@filename` token (3.6). Off = transport only, no composer touch. |

---

## 5. Edge cases

| Case | Handling |
|---|---|
| Drop into a non-focused pane | Allowed. The pane's `cmd:cwd` is the target regardless of focus. (Matches VS Code; the drop event tells us which pane received it.) |
| Drop onto pane header / chrome | Bubbles up — drop into header is a no-op for now. (v2 could route header drops to the pane body.) |
| `cmd:cwd` set to a path the user can't write to | Transport returns the OS error verbatim in the failure toast. We do not pre-check writability — the race window is wider than the check is useful. |
| Drop of a **directory** | Recursive copy (host) / `docker cp -a` (container) / `scp -r` (ssh). Same de-collision rules as files. The 256 MiB cap is the sum of the tree; abort early if exceeded. |
| Drop while the agent is mid-turn | Transport runs; the `@filename` splice waits until the agent enters `IDLE` so it lands in the user's next prompt, not in the middle of streaming output. |
| Two concurrent drops on the same pane | Serialise per pane via a per-pane mutex on the sidecar side. (Prevents two de-collision races picking the same `(1)` suffix.) |
| Drop a file the agent's container can't `ls` (permissions inside the container) | Transport may succeed (root copied it in) but the agent fails to read it. Surface this as a *second* toast on the first agent tool-call failure — not on the drop itself. |
| User cancels mid-transfer (closes pane) | Sidecar transfer is best-effort cancelled; partial file is unlinked. We don't leave half-files for the agent to misread. |
| WSL distro not running | `wsl.rs` starts it on demand (cheap) and proceeds. |
| Docker daemon down | Surface a single toast: "Drop failed: Docker daemon is not running." Do not retry. |

---

## 6. Security

- **Path traversal:** `target_dir` is always `cmd:cwd`, which is set by the sidecar (not user input). `source_path` comes from the OS drag event captured by CEF, not from JS — JS can't fabricate a path. There is no user-controlled string that becomes part of `target/<source>`.
- **Symlink races:** the local copy uses `std::fs::copy`, which dereferences symlinks at the source — sane default. For container/ssh transport the underlying crates follow symlinks at the source side, which matches user expectation ("the bytes I see in Finder, not the link").
- **Sandbox escape:** the agent's process boundary is unchanged. Dropping a file into the agent's CWD does not grant new capabilities — it's bytes on disk the agent could already write itself.
- **Large-file DoS:** the per-file cap (3.8) and the concurrency semaphore (3.7) bound the worst case.

---

## 7. Out of scope

- **Inter-pane file drop** (drag from a File pane to a Terminal pane). Belongs in a separate spec; the drop **source** has different state-capture needs.
- **Drop URLs / drop text.** This spec is files-only. Text/URL drop into the agent composer is the existing browser default and stays as-is.
- **Drop onto a connection-picker.** Dropping a file before the pane has a connection is a v3 idea ("start a terminal here with this file").
- **Two-way DnD** (drag a file out of a pane). Not requested.

---

## 8. Validation

1. Local terminal: drop `report.csv` from Finder → file appears at `pwd`, toast "Copied report.csv to …" within ~200 ms.
2. Local terminal with name collision: drop the same file twice → `report.csv` then `report (1).csv`.
3. Container agent (Docker, claw-style): drop a file → `docker exec <ctr> ls <cwd>` shows it. Agent's next turn references `@report.csv` from the spliced composer token.
4. SSH terminal: drop a file → file lands at the remote CWD; the file owner is the SSH user.
5. WSL terminal: drop a file → file lands inside the distro at the CWD reported by `wsl pwd`.
6. Large file (300 MiB): rejected with size-cap toast within 100 ms.
7. Mid-drop pane close: no half-file left in CWD.
8. Two simultaneous drops of differently-named files: both land; no token-splice interleaving in the composer.

---

## 9. Open questions

- **Do we want `@filename` as the token, or `<filename>`?** Token-form depends on the agent's prompt template. Default `@` because Cursor / Continue / Aider all use it and the muscle memory is there. Tunable via `dnd:agentinserttoken` value (boolean today; could become a string template).
- **Should drag-out be planned now?** Out of scope (§7), but the CEF handler shape will need OnDragStart support for it later. Keep the new module structured so adding OnDragStart is purely additive.
- **Image previews in the DragOverlay?** Nice but not required. Defer to a follow-up.
