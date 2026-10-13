# Default shell: a Settings picker that lists the shells installed, and shows the one in use

**Date:** 2026-10-13
**Status:** implemented (#4837): §4.1–4.3; §4.4, §4.5 and the WSL group are follow-ups. Where the build differs from the proposal, §7 says so.
**Author:** Camper
**Trigger:** Repo owner, 2026-10-13: *"we want to be able to configure the shell users load. can you add a section in settings with best practice ease-of-use for selecting the default shell? We want an elegant way to know what shells are available per platform and make it easy for users to select, also to see the currently set default shell."*
**Related:**
- `SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md` and `SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19.md`: they record `term:localshellpath` and `term:localshellopts` as dead.
- `archive/cross-shell-bootstrap.md`: an earlier priority order, and a `shell:type` meta that was never built.
- `SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` §5: why the agent PtyShell uses `cmd.exe`.
- `SPEC_REMOTES_PANE_2026_10_05.md`: the connection picker and the WSL list.

Code is cited against `main` @ `686723c09`.

---

## 1. Summary

- **The settings exist but do nothing.** `term:localshellpath` and `term:localshellopts` are in `schema/settings.json`, the srv settings struct (`wconfig/types.rs:119-123`) and the frontend types, and `settings-template.jsonc` even shows an example. No code reads them. The constants in `blockcontroller/mod.rs:137-140` are `dead_code`, and the Settings UI dropped them in July for that reason.
- **The shell is hard-coded at spawn** (`blockcontroller/shell/lifecycle.rs:764-771`):
  - **Windows:** `pwsh` if `where pwsh` succeeds, else `powershell`, else `cmd.exe` (`shell/pty.rs:132-162`). This runs on every spawn and never considers Git Bash.
  - **macOS and Linux:** `$SHELL`, else `/bin/bash`.
- **Nothing lists installed shells.** The only enumeration in the codebase is the WSL distro list (`backend/remote/wsl.rs`), used by the connection picker.
- **Proposal:**
  1. **srv honours `term:localshellpath`**, falling back to today's automatic choice when the path is gone.
  2. **A new `shell.list` command** returns the shells found on this machine, with a name, path, version and kind, and says which one new terminals use now and why.
  3. **A "Default shell" row in Settings → Terminal:** a dropdown whose first entry is *Automatic*, naming what that resolves to; then the shells found; then *Custom…* for a path.
  4. **Under it, a line naming the shell new terminals use now,** plus a button to open a terminal with it.
  5. **Later:** a "New terminal with ▸" menu for a one-off shell, using the block-meta key of the same name.

## 2. How it works today

| Path | What it does |
|---|---|
| A terminal pane (`controller: "shell"`, no `cmd`) | The default shell below; integration scripts by the executable's name (`shellintegration.rs:90-104`: pwsh, bash, zsh, fish; anything else gets none) |
| Windows default | `where pwsh` → `pwsh`; else `where powershell` → `powershell`; else `cmd.exe`. Re-run on every spawn. |
| macOS / Linux default | `$SHELL`, else `/bin/bash`. No passwd or `dscl` lookup. |
| `wsl://<distro>` connection | `wsl.exe -d <distro>`: the distro's login shell |
| SSH connection | the remote login shell |
| Agent PtyShell (local, Windows) | always `cmd.exe` (`http_pty_shell.rs:129-152`) |

**Other shell resolvers** (not for terminals, but a list should agree with them):
- `common/src/toolchain_path.rs:284-296`: `$SHELL`, else zsh on macOS or bash.
- `posix_shell.rs` and bashwrap's `locate_bash`: find Git's `bash.exe`/`sh.exe` on Windows (`AGENTMUX_BASH`, PATH, the `git.exe` install, Program Files, LOCALAPPDATA).

**When a change would take effect:** the shell is chosen when the controller spawns, and isn't stored in the block. A new default would apply to new terminals and to re-spawns, not to running ones.

## 3. What good looks like (prior art)

- **VS Code** (`terminal.integrated.defaultProfile.<platform>` plus auto-detected profiles): detects PowerShell 7 and 5, Command Prompt, Git Bash, WSL distros and Cygwin on Windows, and `/etc/shells` elsewhere. It offers them in a picker, marks the default, and lets you add one with a path and arguments. This is the model most users know.
- **Windows Terminal:** generated profiles for PowerShell, Command Prompt, each WSL distro and Git Bash, plus a "Default profile" dropdown on its Startup page. The dropdown shows the icon and name, and the path is only in the profile.
- **macOS Terminal / iTerm2:** "Shells open with: Default login shell | Command (complete path)". Two radio choices, the first naming nothing because the login shell is the system's choice.

What these share, and what this proposal takes from them:
1. **The automatic choice is an explicit option.** It's the default, and it says what it resolves to.
2. **Choices come from detection, not typing.** A custom path is the escape hatch.
3. **Each choice shows a recognisable name first**, with path and version as secondary text.
4. **A broken choice is visible.** A missing path is shown as missing, not silently ignored.
5. **It applies to new terminals,** and says so.

## 4. Proposal

### 4.1 srv: honour the setting (fixes the dead keys)

In the interactive shell branch (`lifecycle.rs:764-771`):
1. **The block's own `term:localshellpath`** meta, if set. Used by the later per-terminal menu (§4.5); it's already in `MetaType`.
2. **Else the global `term:localshellpath`**, from the live config (`controller.rs` `live_settings()`).
3. **Else today's automatic choice**, unchanged: pwsh → powershell → cmd on Windows, `$SHELL` → `/bin/bash` elsewhere.

`term:localshellopts`, or the block's own, replaces the default arguments when set.

**A chosen shell that's gone** (uninstalled, or a typo):
- spawn the automatic one;
- write one line to the terminal saying so (`[AgentMux] Default shell C:\…\nu.exe not found; using pwsh`);
- log it.

Never fail to open a terminal over a stale setting.

**Shell integration:** keep matching by executable name. Two refinements:
- Git Bash's `bash.exe` is already `bash`, but it must be started as `bash.exe --login -i`, from Git's `bin\` (not `usr\bin\`), so `/etc/profile` sets up MSYS paths.
- `cmd.exe` and `nu` get no integration, as today.

### 4.2 srv: `shell.list`, the shells on this machine

A typed command, generated into the frontend stubs like the rest since #4774:

```
shell.list → {
  shells: [{ id, name, path, args?, version?, kind, source, isDefault }],
  automatic: { id, why },                 // what "Automatic" resolves to now
  effective: { id?, path, source, why },  // what a new terminal uses now:
                                          // setting / automatic / fallback-from-missing
}
```

`kind` is one of `pwsh | powershell | cmd | bash | zsh | fish | nu | sh | other`, and drives the icon and integration. `source` says where the entry was found (PATH, a known install location, the registry, `/etc/shells`, the login shell).

**Detection per platform:**

| Platform | Candidates, in order | How |
|---|---|---|
| Windows | **PowerShell 7** (`pwsh.exe`); **Windows PowerShell 5.1**; **Command Prompt**; **Git Bash**; **MSYS2 / Cygwin bash**; **Nushell**, **fish**, other shells on PATH | **pwsh:** PATH, `%ProgramFiles%\PowerShell\*\pwsh.exe`, the Store alias in `%LOCALAPPDATA%\Microsoft\WindowsApps`. **PowerShell 5.1:** `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`. **cmd:** `%ComSpec%`. **Git Bash:** registry `HKLM`/`HKCU\SOFTWARE\GitForWindows\InstallPath` → `bin\bash.exe`, else `git.exe`'s install, else `%ProgramFiles%\Git`, as `posix_shell.rs` already does. **MSYS2 / Cygwin:** `C:\msys64\usr\bin\bash.exe`, `C:\cygwin64\bin\bash.exe`. **Others:** `nu.exe`, `fish.exe` on PATH. |
| macOS | **Login shell** first, then every existing entry of `/etc/shells`, then Homebrew shells not listed there (`/opt/homebrew/bin`, `/usr/local/bin`: zsh, bash, fish, nu) | **Login shell:** `dscl . -read /Users/$USER UserShell`, else `$SHELL`. **Deduplicating:** `/bin/bash` is Apple's 3.2 and Homebrew's is 5.x, so both are listed, with versions. |
| Linux | **Login shell** first, then existing `/etc/shells` entries | **Login shell:** `getent passwd $USER`, else `$SHELL`. **Deduplicating:** by resolved path (`/bin/bash` and `/usr/bin/bash` on usrmerge systems are one). |

**Versions:** ask each candidate for its version, with a short timeout (`pwsh -NoProfile -Command $PSVersionTable.PSVersion`, `bash --version`, and so on), all in parallel. Cache the whole list for 30 s like the WSL list, and refresh on demand when the settings row mounts. A version that doesn't answer in time is left out; the shell still lists.

**WSL distros aren't local shells.** They're connections (`wsl://`), and the connection picker already lists them. The list may include them as a separate group whose choice sets the new-terminal *connection*. That's a bigger semantic change (§6), so it's phase 2.

### 4.3 Settings → Terminal: the "Default shell" row

```
Default shell
  [ ⚙ Automatic: PowerShell 7.6.6                         ▾ ]
     Automatic: PowerShell 7.6.6        C:\Program Files\PowerShell\7\pwsh.exe
     ──────────────
     PowerShell 7.6.6                   C:\Program Files\PowerShell\7\pwsh.exe
     Windows PowerShell 5.1             C:\Windows\System32\…\powershell.exe
     Command Prompt                     C:\Windows\System32\cmd.exe
     Git Bash 5.2.37                    C:\Program Files\Git\bin\bash.exe
     Nushell 0.98                       C:\Users\…\nu.exe
     ──────────────
     Custom…
  New terminals open with PowerShell 7.6.6 (automatic: the newest PowerShell found).
  [ Open a terminal with it ]        Applies to new terminals; open ones keep their shell.
```

- **The dropdown's first entry is "Automatic: <name>"**, the default, writing `null` (the key cleared), the same pattern as `term:durable` in `terminal-section.tsx:223-236`. Then the detected shells, each showing its name and version, with the path in secondary text. Then **Custom…**.
- **Choosing a detected shell** writes its path to `term:localshellpath` with `set()` (`settings-controls.tsx`). Its default arguments are already known, so `term:localshellopts` stays empty unless the user changes it.
- **Custom…** reveals a path field with **Browse…**, which uses the host's native file picker. It also reveals an arguments field for `term:localshellopts`. A path that doesn't exist shows a red "not found" on the row.
- **The status line comes from `shell.list`'s `effective`.** It names what new terminals use now, and why: "automatic: the newest PowerShell found", "your setting", or "your setting (C:\…\nu.exe) wasn't found; using PowerShell 7".
- **Open a terminal with it** creates `{view: "term", controller: "shell"}` in the current tab, so the choice can be tried at once.
- **Platform:** the row shows only this machine's shells. `settings.json` belongs to the install on this machine, so a path never applies to another OS.
- **Search keywords** for the settings index: shell, default shell, pwsh, powershell, bash, zsh, git bash, cmd, `term:localshellpath`.

### 4.4 The terminal's own hover panel

The stats badge's hover panel (`element/blockstats.tsx`) can name the shell a terminal runs ("Shell: PowerShell 7.6.6"), next to the new Renderer row. That gives an answer to "what am I in?" without opening Settings. srv knows it at spawn; send it with the controller status.

### 4.5 Later: a one-off shell for one terminal

A **New terminal with ▸** submenu, in the tab bar's new-pane menu and the command palette, lists the same shells. It creates the block with `term:localshellpath` in its meta (§4.1 step 1). That's the VS Code "+ ▾" pattern.

## 5. Edge cases

- **The PowerShell 7 Store alias** (`WindowsApps\pwsh.exe`) is a reparse point that can't always be read for a version. List it and leave the version out.
- **The agent PtyShell** keeps `cmd.exe` on Windows regardless of this setting (§2). PSReadLine's `ESC[6n` query hangs with no terminal emulator attached. Say so in the row's description, so nobody expects agents' shells to follow it.
- **Remote panes (SSH, WSL)** keep their login shell. The setting is for local terminals; the row's description says "local terminals".
- **Durable terminals** (`term:durable`) that re-attach keep the shell they started with.
- **`conn:shellpath`** (a per-connection key in `connections.json`) is just as dead. It's out of scope here: either wire it for SSH in a follow-up or remove it, and record the decision in `settings-defaults.test.ts`'s `UNREAD` list.

## 6. Plan (steps 1 and 2 done)

1. **srv:**
   - `term:localshellpath` and `term:localshellopts`, block meta then global, with the missing-path fallback and the notice (§4.1).
   - `shell.list` with detection per platform and the 30 s cache (§4.2).
   - Tests: Windows detection with a fake filesystem and registry; `/etc/shells` parsing; deduplication; the fallback.
2. **Frontend:** the "Default shell" row (§4.3). Tests: Automatic writes `null`; a detected choice writes its path; Custom validates; the status line follows `effective`.
3. **Then:** the hover panel's Shell row (§4.4), and the "New terminal with ▸" menu (§4.5).
4. **Phase 2, if wanted:** WSL distros in the picker as a default *connection* for new terminals.

## 7. As built

- **srv:** `backend/shells.rs` (detection, `resolve_for_spawn`, the 30 s cache), `rpc_types/shells.rs`, the `shell.list` handler in `server/app_api/shells.rs`. The spawn path in `blockcontroller/shell/lifecycle.rs` honours the block's `term:localshellpath`, then the global one, then the automatic choice. `term:localshellopts` (the block's, else the setting's) is appended after the integration arguments rather than replacing them, so shell integration keeps working. The old `detect_local_shell_path_windows` is gone; `shells::automatic` does the same with full paths.
- **A missing shell** is logged (`configured shell not found; using the automatic one`) and shown in Settings as a warning under the dropdown. A line written into the terminal, as §4.1 proposed, isn't built.
- **Settings:** `sections/default-shell.tsx`, logic in `default-shell-model.ts` (tested), as the first row of Terminal. Custom… takes a full path or a name on PATH, and space-separated arguments. There is no Browse… button: the frontend has no native file-picker API today.
- **Not built yet:** the Shell row in the hover panel (§4.4), "New terminal with ▸" (§4.5), WSL distros in the list, and version detection for the Store's `pwsh` alias where it can't be read.
