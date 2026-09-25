# SPEC: status bar host popover — drop the "Instance" row, add "open in file manager"

**Author:** Agent4
**Date:** 2026-09-25
**Status:** proposed — claims verified against `main` @ `3cd1bd188` (2026-09-25)

---

## 1. Questions this answers

1. What is the **Instance** row in the status bar's network/system (host) popover?
2. Why does it read only a version (e.g. `v0.55.37`)?
3. Add a system link that opens the OS file browser, on Windows, macOS and Linux.

---

## 2. What "Instance" is today

The row lives in `frontend/app/statusbar/HostPopover.tsx` (the "Instance Info" block,
directly under OS / IP):

```tsx
<span class="status-bar-popover-label">Instance</span>
<span>{props.hostInfo()!.instanceId}</span>
```

`instanceId` comes from the `get_host_info` IPC command
(`agentmux-cef/src/commands/platform.rs`, `get_host_info`):

```rust
let version = env!("CARGO_PKG_VERSION");
...
"instanceId": format!("v{}", version),
"version": version,
```

So the row is a **placeholder that duplicates `version`**. It is not an identifier of
anything. Nothing else in the frontend reads `HostInfo.instanceId` (grep: the type
declaration and the one render site), and `hostType: "host"` is likewise unread.

### 2.1 Why it reads a version — it is the backend's meaning of "instance"

In the backend, "instance" historically means **the version-scoped install**, not a
running process:

- `agentmux-srv/src/config.rs`: `--instance` — "Instance identifier (used for
  multi-version coexistence)", default `"default"`. Both spawners pass `v<version>`:
  `agentmux-launcher/src/srv_spawner.rs` (`let instance_id = format!("v{}", version)`)
  and `agentmux-cef/src/sidecar.rs` (`let version_instance_id = format!("v{}", current_version)`).
  The name is now historical: data isolation moved to *channels*
  (`~/.agentmux/channels/<channel>/`, `agentmux-common/src/data_paths.rs`, whose own
  doc comment says `instance_dir` "is now the *channel* root, not the *version* root").
- `agentmux-srv/src/backend/lan_discovery.rs` (doc comment on `mdns_instance_label`):
  "`instance_id` is the *version*". This caused a real bug on 2026-09-06 — every host on
  one release registered the identical mDNS name — and the fix made the mDNS service
  label `agentmux-<hostname>-<port>` the unique key, keeping `instance_id` (the version)
  in the TXT record.

`get_host_info` copied that convention. The result is a label ("Instance") whose value is
the same string as the app version chip and the Backend panel's version line
(`BackendStatus.tsx:151` already notes the version "matches the app version already
shown in the Instance panel").

### 2.2 It is also inconsistent with the rest of the UI

- The **LAN peers** list in the same popover shows `inst.hostname || inst.instance_id`.
  For a peer, `instance_id` is read from the mDNS TXT record's `instance_id` property
  (`lan_discovery.rs`, around line 947; the local variable there is *named* `peer_id`,
  but it is that TXT property) — and that property is the **version** again. There is
  no separate per-process identifier in the LAN layer. What actually makes a running
  instance unique on the network is the mDNS service label `agentmux-<hostname>-<port>`
  (`mdns_instance_label`), not any ID field.
- `InstancePanel.tsx` ("Instance panel", opened from the version chip) means something
  else again: the list of open windows in this process.

Two meanings of "instance" in one status bar (version-scoped install vs. the window
list), and a version-valued field named `instance_id` throughout the LAN layer.

> Correction (2026-09-25): an earlier draft of this section claimed peers show a real
> per-process `peer_id`. That was wrong — misread from a variable name. Verified against
> `lan_discovery.rs` (TXT record built at ~line 411, read back at ~line 947).

---

## 3. Decision — drop the Instance row

**Decided 2026-09-25 (user):** remove the `Instance` row and do not replace it. Users
already have both values it could stand for, one click away in the version chip's
panel (`frontend/app/statusbar/InstancePanel.tsx`):

- **Version** — `v{about().version}`, with a copy button;
- **Channel** — `about().channel`, with a copy button (shown whenever a channel is set).

Both come from `get_about_modal_details`. Repeating them in the host popover would
just be a second copy of the same data.

Keep **PID** and **Data** rows as they are. For installed and portable builds the Data
path is `~/.agentmux/channels/<channel>/data`, so the channel name is visible there too;
for `task dev` builds it is `~/.agentmux/dev/<branch>/…` (channel `dev-<branch>`), per
`agentmux-common/src/data_paths.rs`.

### 3.1 Backend change

In `get_host_info` (`agentmux-cef/src/commands/platform.rs`), remove `"instanceId"` and
`"hostType"`. Both are unread: in `frontend/`, the only reader of `instanceId` is the row
being removed, and `hostType` has no reader. No new fields.

### 3.2 Frontend change

`HostPopover.tsx`: delete the `Instance` row and drop `instanceId` and `hostType` from the
`HostInfo` type. The divider above the old "Instance Info" block stays, now leading into
PID / Data.

### 3.3 Rejected

- Replacing the row with Version + Channel rows — drafted earlier the same day, dropped
  because `InstancePanel` already shows both (see above).
- Keeping `Instance` with a different format — any `CARGO_PKG_VERSION`-derived value is
  a duplicate.
- A per-process ID row — none exists (§2.2); adding one would change the mDNS TXT record.
  PID + ports already tell two local processes apart.

---

## 4. Proposal — "Open in file manager" link

### 4.1 UX

Turn the existing **Data** row into an action: the path text stays, a small
folder icon-button follows it. Tooltip (`data-tip`, per the status bar's own tip
convention — not the native `title=`):

- Windows: "Show in File Explorer"
- macOS: "Reveal in Finder"
- Linux: "Open in file manager"

Label via the existing `isMacOS`/`isLinux` helpers already imported in `HostPopover.tsx`.
Also add a second, optional link for the **Config** dir if it is cheap (see §7
question 1); Data is the required one.

Failure UX: on IPC error, show the error inline in the popover using the same
warning-colour row the LAN error uses. Do not swallow it — on a headless Linux box with
no handler, silence would look like a dead button.

### 4.2 New IPC command

Do **not** reuse `open_in_editor`. Its job is "open this file for editing": on Windows it
runs `explorer <path>`, but on macOS and Linux it first tries `code`/`cursor`/`zed`/
`subl`/`atom` and only falls back to `open`/the Linux handler if none is installed — so
on a machine with VS Code, a folder link would open VS Code, not the file browser.
It also takes an arbitrary renderer-supplied path, which §4.2's enum avoids. Do not reuse `open_external` — it deliberately allow-lists only
`http(s)://`, `devtools://`, `vscode://` and refuses everything else, which is the
right posture for it.

Add `open_in_file_manager` to `agentmux-cef/src/commands/platform.rs`, registered in
`ipc.rs` next to `open_in_editor`.

**Do not take a path from the renderer.** Take a closed enum instead:

```jsonc
{ "target": "data" | "config" }
```

The host resolves the path itself from `state.version_data_dir` / `version_config_dir`
(the same values `get_data_dir` / `get_config_dir` return). That removes the
"renderer can ask the host to open any path" primitive entirely; there is no path
validation to get wrong and nothing to shell-inject. Unknown target → error.

If the dir is `None` (not initialized) → the same "Data dir not initialized yet" error
`get_data_dir` returns. If the dir does not exist on disk → error, do not create it.

### 4.3 Per-platform launch

Always spawn the binary directly with the path as a single argv element — never through
`cmd /C start`, `sh -c`, or string-concatenated commands (the existing code already
avoids this for the same reason; see the comment in `open_url_in_default_browser`).

| OS | Command | Notes |
|---|---|---|
| Windows | `explorer.exe <path>` | Opens a **directory**. Explorer is widely reported to exit with code 1 even on success, so never treat its exit code as a result — `spawn()` and fail only if the spawn itself errors (same as `open_in_editor` does today). Normalise `/` → `\` before spawning: explorer is also widely reported to ignore a forward-slash path and open its default folder instead. Both behaviours are to be confirmed in manual testing (§6), not assumed. |
| macOS | `open <path>` | For a directory this opens it in Finder. (`open -R <path>` reveals a *file* selected in its parent — only needed if we later link to a file.) |
| Linux | `xdg-open <path>` | If `xdg-open` is not installed (spawn fails with not-found), try `gio open <path>`. If neither exists, return an actionable error ("no file manager handler found") — the popover already displays the path, so the user is not stranded. |

Use `spawn()` and detach; never `wait()` on the UI thread. Set `stdin/stdout/stderr` to
null so a file-manager's output cannot attach to the host's console.

**Known limitation:** "spawn succeeded" is not "a window opened". An `xdg-open` that is
installed but has no directory handler (e.g. a minimal/headless Linux box) spawns fine
and fails later, asynchronously; the command cannot report that without waiting on the
child. Accept this — waiting would block, and the path is visible in the row regardless.
Do not claim in UI copy that the folder was opened.

`#[cfg(target_os = ...)]` gating follows the existing pattern in
`open_url_in_default_browser`; add a final `#[cfg(not(any(windows, macos, linux)))]`
arm returning `Err("unsupported platform")` so the function never falls off the end
with an implicit success.

### 4.4 Frontend wiring

In `HostPopover.tsx`, call through the same `invokeCommand` used for `get_host_info`:

```ts
await invokeCommand("open_in_file_manager", { target: "data" });
```

Put the button inside the existing `<Show when={props.hostInfo()}>` block, next to the
Data path. When `get_host_info` fails (the file's `catch` sets `hostInfo` to `null`), the
whole block — Data row included — is already hidden, so no extra "IPC unavailable"
handling is needed.

No type registry to update: `invokeCommand` (`frontend/app/platform/ipc.ts`) takes the
command name as a plain `string`.

---

## 5. Security notes

- Renderer → host, so this is a privilege boundary. The `target` enum design in §4.2
  is the control: the renderer cannot influence the path opened.
- No shell is involved on any platform; the path is one `argv` element.
- The opened location is user-owned app data; the action opens a window for the user
  and returns nothing sensitive to the renderer.
- Do not extend this command to accept arbitrary paths later without a separate spec —
  that would need canonicalisation, a root allow-list, and symlink handling.

---

## 6. Tests

- **Rust unit tests** (existing `#[cfg(test)]` module in `platform.rs`). Keep the
  command testable without an `AppState` or a real file manager by splitting it into
  two pure helpers:
  - `resolve_file_manager_target(target: Option<&str>, data_dir: Option<&str>, config_dir: Option<&str>) -> Result<PathBuf, String>`
    — tests: unknown target → `Err`; missing target → `Err`; dir `None` → `Err` with
    the "not initialized" message; nonexistent dir → `Err`; existing temp dir → `Ok`.
  - `file_manager_command(path: &Path) -> (&'static str, Vec<OsString>)` — per-OS,
    `#[cfg]`-gated assertions (Windows: `explorer.exe` + backslash-normalised path;
    macOS: `open`; Linux: `xdg-open`). The Linux `gio` fallback is a spawn-time retry, so
    it is covered by manual testing, not this helper.
- **`get_host_info`**: assert the JSON no longer has `instanceId` / `hostType`.
- **Frontend** — new file `frontend/app/statusbar/HostPopover.test.tsx` (none exists
  today; `TokenBreakdownPopover.test.tsx` next to it is the pattern). Cases: no `Instance`
  label is present; the button renders with the
  Data row; clicking calls `invokeCommand("open_in_file_manager", { target: "data" })`;
  a rejection renders the inline warning row; the tooltip string per OS.
- **Manual verification on all three OSes is required before merge** — the launch
  behaviour is OS-specific and cannot be proven by unit tests. Windows first (this is
  the primary platform); macOS and Linux need a real machine or CI runner with a
  desktop session. State explicitly in the PR which OSes were manually verified; do not
  claim macOS/Linux verified if they were only compiled.

---

## 7. Open questions

1. Do we want a Config-dir link as well, or Data only?
2. Rename the `InstancePanel` / "Instance panel" wording too, now that the popover no
   longer uses "Instance"? Out of scope here, but worth a follow-up.

(Resolved 2026-09-25: drop the Instance row, no replacement; no per-process ID.)

---

## 8. Implementation checklist

- [ ] `get_host_info`: remove `instanceId` and `hostType`.
- [ ] `HostPopover.tsx`: update `HostInfo` type; delete the Instance row; add the file-manager button on the Data row.
- [ ] `platform.rs`: `open_in_file_manager` + `resolve_file_manager_target` + `file_manager_command`; register in `ipc.rs` next to `open_in_editor`.
- [ ] Tests per §6, including the new `HostPopover.test.tsx`.
- [ ] Manual check on Windows, macOS, Linux; PR states which were actually exercised.
- [ ] `task docs:index` to regenerate `docs/specs/INDEX.md`; flip this spec's **Status** to `implemented — PR #NNNN`.
- [ ] PR body includes the `<!-- agentmux:agent_id=agent4 -->` tag.
