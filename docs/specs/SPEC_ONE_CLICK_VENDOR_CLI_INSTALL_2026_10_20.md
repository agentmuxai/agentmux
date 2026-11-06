# SPEC: One-click install for the six vendor-installed agent CLIs

**Date:** 2026-10-20
**Status:** active — operator's answers to §7 given through AgentX (2026-11-05); built in #138 by Camper-2: §4.1 to §4.4 for the new-agent panel, with the approval in srv's approval window; what is not built is in §8
**Context:** the security design for Step B of [`SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md`](SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md) §4.4.
**Related:** [`SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md`](SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md) (the
winget / Homebrew / pkexec one-click install this reuses),
[`SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md`](SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md) (`InstallConfirm`, the
consent step), [`SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`](SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) (`install.start`
and the `install_chunk` stream), [`SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md`](SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md)
(the panel under a harness card, where the button goes),
[`SPEC_NINE_NEW_HARNESSES_2026_10_14.md`](SPEC_NINE_NEW_HARNESSES_2026_10_14.md) (goose, Grok, Aider, Muse).

## 1. What this is

Six harnesses aren't on npm, so AgentMux can't install them into its own isolated folder. Today the panel under such a
card shows the vendor's install command with **Copy**, a link to the vendor's instructions and **Check again**:

| Harness | Windows | macOS and Linux |
|---|---|---|
| Kimi Code CLI | `pip install kimi-cli` | `pip install kimi-cli` |
| Antigravity | `irm https://antigravity.google/cli/install.ps1 \| iex` | `curl -fsSL https://antigravity.google/cli/install.sh \| bash` |
| goose | `irm https://github.com/aaif-goose/goose/releases/download/stable/download_cli.ps1 \| iex` | `curl -fsSL …/download_cli.sh \| CONFIGURE=false bash` |
| Grok Build | `irm https://x.ai/cli/install.ps1 \| iex` | `curl -fsSL https://x.ai/cli/install.sh \| bash` |
| Aider | `irm https://aider.chat/install.ps1 \| iex` | `curl -LsSf https://aider.chat/install.sh \| sh` |
| Muse Code | `irm https://dev.meta.ai/install.ps1 \| iex` | `curl -fsSL https://dev.meta.ai/install.sh \| sh` |

This spec adds a **Run** button beside **Copy**: AgentMux runs that installer for the user, streams its output into the
panel, and re-detects the CLI when it ends.

Five of the six run a script fetched from the vendor's site at the moment of install, unpinned. That is remote code
execution by design: it is what the user would do by pasting the command, and the user is choosing to trust the vendor.
The design below is about making sure **only the user can start it, only the vendor's own command runs, and the user
sees exactly what ran**. It does not make the installers safe, and the panel must not suggest it does.

**Not in scope:** isolating these CLIs (the vendors' installers write into the user's account and `PATH`; the panel
already says so), pinning vendor versions, installing Python for Kimi (the existing system-tool step does that),
uninstall.

## 2. What exists today (read 2026-10-20)

- **The six commands live only in the frontend**: `windowsInstallCommand`, `unixInstallCommand` and `docsUrl` in
  `frontend/app/view/agent/providers/catalog.ts` (types in `providers/types.ts:79-82`). srv never runs them; it only
  echoes the frontend's text back as the `CliMissingOnPath` install hint (`cli_handlers.rs:111-134`). No version pin and
  no checksum for any of them. srv's provider records carry `known_install_paths` for each vendor's folder
  (`providers.rs:417-504`; none for Kimi) and a "checked against" version note that nothing enforces.
- **The system-tool one-click install is the model to copy** (`server/system_install_handlers.rs`):
  `toolchain.install_system_tool {toolId}` takes only an id; the command is built in srv from a compiled catalog
  (winget ids, brew formulas, Linux package lists); it is spawned as an argv list, never through a shell (a test asserts
  the program is never `sh`/`bash`/`cmd`/`powershell`); `sanitize_external_command` strips `AGENTMUX_*` from the child's
  environment; output streams as `install_chunk` events; `InstallConfirm` shows the exact server-built command and the
  user clicks **Install** first.
- **But its consent is enforced only in the frontend.** The handler ignores its caller context; WebSocket `rpc`
  messages reach the engine with no per-command authorization (`websocket.rs:712-714`); and every pane process inherits
  `AGENTMUX_LOCAL_URL` and `AGENTMUX_AUTH_KEY` (`pane_env.rs:56-57`). So a process in an agent's pane could call it
  without any click. See §6.
- **The npm path** (`install.start`) is started by the user from the card, and by `agent.open` (MuxAPI) for a missing
  npm CLI using srv's own package and pin. Note for §6: `install.start` checks the `npmPackage` the caller sends only for
  being non-empty, and installs it even when it doesn't match the provider (`install_handlers.rs:456, 762-766`). npm runs
  package install scripts (no `--ignore-scripts`).

## 3. Threats, and what each one requires

| # | Threat | Requirement |
|---|---|---|
| T1 | A caller makes srv run a command of its choosing. | srv runs only a command from a catalog compiled into srv, chosen by provider id. No command text, URL, argument or environment ever comes from the caller. |
| T2 | An agent (or anything else holding the local auth key) starts an install without the user. An agent with a shell could run `curl … \| bash` itself, but then its own CLI's permission prompt applies; an AgentMux RPC that skips that prompt would make AgentMux the way around it. | The install runs only after a user approval that srv itself checks, through a channel a pane process can't reach (§4.3). A frontend-only confirm is not enough. |
| T3 | The vendor script, or a redirect, serves something other than the vendor's installer. | HTTPS only; an allowlist of hosts per provider, redirects included; the script is downloaded first, its SHA-256 and size are shown and logged, and *that file* is what runs. No hash pinning (it would break on every vendor release); the user is trusting the vendor, and AgentMux records what that trust ran. |
| T4 | The installer escalates, or waits forever on a prompt. | AgentMux never elevates. stdin is closed, so a `sudo` or interactive prompt fails instead of hanging. A failure says so and shows the manual command. A timeout ends a stuck install. |
| T5 | The installer reads AgentMux secrets from its environment. | `sanitize_external_command`, as the system-tool install does. Nothing else is added to the environment except what the catalog entry names (goose's `CONFIGURE=false`). |
| T6 | The user can't tell what happened. | The consent step shows the exact command, the download URL and the note that this installs outside AgentMux's isolation; the log shows the hash, every line of output and the exit code; on failure the panel says the installer may have left files and links the vendor's instructions. |
| T7 | Two installs of the same CLI at once, or no way to stop one. | One install per provider at a time; **Cancel install** kills the process tree. |

## 4. Design

### 4.1 One catalog, in srv

Move the six install recipes from `catalog.ts` into srv, next to `known_install_paths`, as a compiled table
`VendorInstall { provider_id, unix: Recipe, windows: Recipe, docs_url, allowed_hosts }`, where `Recipe` is one of:

- `Script { url, interpreter: Bash | Sh | PowerShell, env: &[(&str, &str)] }`
- `Pip { package }` (Kimi): runs `<python> -m pip install --user <package>` with the Python the toolchain resolver
  found, argv only.

The frontend gets the display text from a read-only RPC (`toolchain.vendor_install_info {providerId}` → command text,
URL, docs link), so **Copy** and **Run** show the same command and there is one source of truth. `catalog.ts` keeps the
docs link only. This also fixes the drift risk the isolation spec noted: the hint srv shows on a launch failure comes
from srv's own table instead of from the frontend.

### 4.2 Running it

`toolchain.install_vendor_cli {providerId, approvalId}` (new; §4.3 for `approvalId`):

1. Look up the provider's recipe for this OS; refuse anything else.
2. For a `Script`: download the URL with srv's HTTP client (HTTPS only, every redirect's host in `allowed_hosts`,
   size cap of a few MB, timeout) into a temp file in AgentMux's own temp folder. Compute SHA-256. Emit the URL, final
   host, size and hash as the first `install_chunk` lines and log them.
3. Spawn the interpreter on that file as argv, never a pipe and never a shell string:
   - Unix: `bash <file>` or `sh <file>`, with the recipe's env.
   - Windows: `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <file>`. The process-scoped
     `Bypass` is the decision §7.1 asks to confirm: a downloaded `.ps1` is blocked by the default policy on many
     machines, and `irm | iex` (what the user would paste) is not subject to it either.
   - stdin closed; `sanitize_external_command`; no elevation.
4. Stream stdout and stderr as `install_chunk` on `install:<sid>`, end with `done` and the exit code, delete the temp
   file.
5. Re-detect through `known_install_paths` and `ResolveCliCommand` (the running srv's `PATH` is stale until restart),
   and report found / not found. Not found after exit code 0 means "installed, but AgentMux can't see it yet", with
   **Check again** and the restart hint the system-tool install already uses.
6. A timeout (proposal: 10 minutes) and **Cancel install** kill the process tree. One session per provider.

### 4.3 Only the user can approve it

The requirement (T2) is that srv, not the frontend, knows a person approved this install, through a channel no pane
process can drive. The proposal, to confirm in implementation:

- **Use the approval-window pattern AgentMux already has** (`SshApprovalWindow`, `CredentialApprovalWindow`,
  `MemoryAdoptionApprovalWindow`): the panel's **Run** asks srv to create a pending approval for
  `{providerId, recipe, url}`; srv shows its own approval surface naming the command and URL; only an approval from that
  surface marks it approved; `install_vendor_cli` runs only with an `approvalId` that srv approved, for that provider,
  within a short TTL, once.
- **Verify before relying on it** that an approval can't be granted over the same WebSocket RPC surface a pane process
  can reach with `AGENTMUX_AUTH_KEY`. If it can, that's a gap in the approval windows themselves and needs fixing first
  (§6).
- No MuxAPI / MCP tool starts this. `agent.open` keeps auto-installing npm CLIs only; for the six it keeps failing with
  the install hint, as today.

### 4.4 The panel

In the vendor-install step of the panel under the card (`AgentCreatePanel.tsx`, `agent-create-vendor-install`):

- Keep the command with **Copy** and the vendor link. Add **Run** as the main action, with **Check again** beside it.
- **Run** opens the approval (§4.3). Its text: what will run (the exact command and URL), that it comes from the vendor,
  that it installs into the user's account and `PATH` outside AgentMux's isolation, and that it may ask for nothing
  interactive (if it needs a password, it fails and the manual command still works).
- While it runs: the same steps and **Details** log as the npm install; **Cancel install**; closing the panel doesn't
  stop it, the card shows **Installing…** (the npm path's behaviour).
- On success: the agent's fields follow, as after an npm install. On failure: the last lines of output, the note that
  the installer may have left files, the vendor link and **Try again**.

The Toolchain pane's row for these six gets the same **Run** through the same approval.

## 5. Tests

- srv: the recipe table covers exactly the six and each OS; `install_vendor_cli` refuses an unknown provider, a missing,
  expired, reused or other-provider `approvalId`; argv never contains a shell or a pipe; a redirect to a host outside
  `allowed_hosts` aborts before anything runs; an `http://` URL is refused; the downloaded file's hash is in the first
  chunk; the env is sanitized; cancel kills the child; the temp file is gone after success, failure and cancel.
- frontend: **Run** shows the server's command text (not the catalog's); the approval is required; the panel's states
  (running, cancelled, failed, installed-but-not-found, found).
- Live, once per OS: install goose, Aider and Kimi from a clean account; check the hash line, the re-detection without
  restart, and the failure path with the network off.

## 6. Gaps found while writing this (fix separately, before or with this)

1. **The system-tool one-click install has no server-side user gate** (§2): consent lives only in the frontend, and a
   pane process can reach the RPC. It runs package-manager installs of fixed packages, so the impact is small, but it
   should get the same approval as §4.3.
2. **`install.start` installs whatever `npmPackage` the caller sends** (§2). It should take the package from srv's
   provider registry, as `agent.open`'s auto-install already does, and refuse a mismatch.
3. **The isolation spec's Step A** assumed the six's install commands would stay in the frontend; §4.1 moves them, so
   that spec's description should point here once this lands.

## 7. Open questions

1. **PowerShell execution policy** (§4.2): process-scoped `-ExecutionPolicy Bypass` on the downloaded file, or run it
   the way the vendor documents (`irm | iex` via `-Command`, which loses download-then-run and the hash)? Recommendation:
   `Bypass` on the file.
2. **Kimi:** `pip install --user` into the resolver's Python, or `uv tool install kimi-cli` when `uv` is present (closer
   to an isolated install)? Recommendation: `pip --user` first, as the vendor documents.
3. **Approval surface:** a separate approval window like the SSH and credential ones, or a srv-mediated confirm inside
   the panel? Only the first is out of an agent's reach today, pending the check in §4.3.
4. **Timeout:** 10 minutes, or per provider?

## 8. As built (#138, 2026-11-05)

Answers to §7, from AgentX: (1) process-scoped `-ExecutionPolicy Bypass` on the downloaded file; (2) Kimi through
`pip install --user` first; (3) the approval inside the pane, built on the pane-modal primitive of #18804 and #17125;
(4) ten minutes, with a per-provider override.

**Built**
- **One catalog in srv** (`server/vendor_install_catalog.rs`): the five recipes (Kimi, Antigravity, Grok Build, Aider,
  Muse Code) for each OS, the hosts a script may come
  from, the time limit (`timeout_secs`, ten minutes each). `toolchain.vendor_install_info {providerId}` gives the panel
  the vendor's command for this OS, so **Copy** and **Run** show the same text; it answers `available: false` for any
  other provider, and the panel then keeps its copy-only step.
- **`toolchain.install_vendor_cli {providerId}`** takes an id and nothing else (T1). The same `install_chunk` events
  and `install.cancel` as the npm and system-tool installs; one install per provider (T7).
- **Only the user can start it (T2).** srv asks in the approval window (`ask_user`), the one the system-tool install
  already uses and no pane process can answer. The question names the vendor's command, what AgentMux does, and that
  it installs outside AgentMux's isolation. With no window, no answer or a no, nothing is downloaded or run. §6 gap 1
  is therefore already closed for the system-tool install, which has had this gate since it was written.
- **Download, then run that file (T3).** HTTPS only, no credentials or odd port in the address, every redirect's host
  on the provider's list, 2 MB cap, 60 s limit. An empty answer or a web page is refused. The first lines of the
  run's log are the vendor's command, the URL, the final host, the size and the SHA-256, and srv logs the same.
- **Run (T4, T5).** Argv only (`bash <file>`, `sh <file>`, `powershell -NoProfile -NonInteractive -ExecutionPolicy
  Bypass -File <file>`; for Kimi `<python> -m pip install --user kimi-cli`, with the Windows `py` launcher first). stdin
  closed, no elevation, `sanitize_external_command`, the recipe's own environment only (none of the five sets one today).
  The downloaded file is removed when the run ends, however it ends.
- **Cancel and time limit** end the installer's whole process group (`taskkill /T` on Windows).
- **Re-detection** after a clean exit through the provider's known install folder and PATH. Found: a line says where.
  Not found: the restart note the system-tool install uses, and the panel shows **Check again**.
- **The panel**: **Run** beside **Copy** and the vendor's link, the same steps and **Details** as the npm install,
  **Cancel install**, **Try again**. A failure says the installer may have left files; a refusal says nothing was run.
  Closing the panel doesn't stop the run (the session outlives it, as the npm install's does).

**Found while building**
- **goose is not in the table.** AgentMux now downloads goose itself (#6590, Lazo: a pinned release, its SHA-256
  checked, its own folder, an Install button), so the vendor script would be a second way to install one CLI. The
  managed install is the end state: Lazo's later changes do the same for Aider and Kimi (a pinned `uv`) and others, and
  each one removes that CLI's row from this table. A test (`a_cli_agentmux_downloads_itself_is_not_in_the_vendor_table`)
  fails if a provider is in both this table and `cli_download::SPECS`, so a CLI can't show two installs.
- goose's Windows command in `catalog.ts` (`releases/download/stable/download_cli.ps1`) answered 404; the release has no
  such file. goose documents `raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1`, which `catalog.ts` now
  uses (it is the command for a copy the person installs on their own).
- Every script is under 60 KB, and the vendor sites answer without a redirect, so each provider's list is one host.

**Not built**
- **The approval inside the pane** (§7.3's answer). The approval window is the same surface as the system-tool
  install's today. When the pane-modal primitive lands, both move to it together; nothing in the RPC changes.
- **The Toolchain pane's row** gets **Run** through the same RPC in its own change.
- **Moving the commands out of `catalog.ts`** (§4.1): `catalog.ts` still carries them, for the hint srv echoes on
  a launch failure. The panel shows srv's text; the hint is the remaining copy, to be read from srv's table.
- **Live runs** of each installer on each OS (§5): the srv tests cover the download policy, the approval, argv, the
  environment and cancel with stand-in installers; no vendor installer was run for this change.
- **§6 gaps 2 and 3** are their own tasks.
