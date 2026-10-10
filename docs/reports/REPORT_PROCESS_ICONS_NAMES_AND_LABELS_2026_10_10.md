# AgentMux's processes in the OS task manager: icons, names and pane labels on Windows, macOS and Linux

**Status:** implemented — §9 items 1 to 4 are built: Windows icons and names, the build in the host name and macOS helper icons (#4595), the Linux srv name (#4596), and Tower's labelled AgentMux rows (#4598, and renderers named by the window or pane they serve in the PR that sets this status). Item 5 is not built: Tower shows the same. Item 6 is not built (§9). §8.2's per-type CEF names on Linux were dropped (§8.2).
**Date:** 2026-10-10 | **Version:** main at 0.59.18 (`f2da6d823`); Windows measured on 0.59.16 and 0.59.17 portables and four `task dev` builds running side by side. macOS and Linux findings are from the packaging scripts, not from a running machine (§10).
**Requested by:** repo owner: "in the background processes … AgentMux Server, and all the agentmux-bashwrap, mcp, etc, we want to use our icons, currently they are using generic ones"; then "there are many entries with just 'AgentMux' … are we able to get more information in the name?", "do they relate to actual panes in the agentmux instances? can they be labeled?" and "can we ensure the icons are supported on 3 platforms?"
**Author:** Loap.
**Related:** `scripts/inject-exe-icon.sh`, `crates/{cef,launcher,srv}/build.rs`, `scripts/package-macos.sh`, `scripts/build-appimage-linux.sh`, `docs/specs/SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md` (§ open question 2: whether Tower splits out CEF helpers).

---

## 1. Summary

- **Windows, icons.** Three of AgentMux's own executables carry no icon: `agentmux-srv`, `agentmux-mcp`, `agentmux-bashwrap`. The last two carry no name either, so Task Manager shows the bare file name. The fix is compile-time and small (§5).
- **Windows, the many "AgentMux" rows.** They are the CEF host and its subprocesses: per instance one main process, one GPU process, network/storage/audio(/video) utility processes, and one renderer per window or browser pane. Task Manager names a process after its exe file, and all of these run the same exe, so Task Manager can't tell them apart (§6). What can change: the version/build in the name, which separates instances; a marker on each subprocess's command line for the Details tab; and the per-type and per-pane breakdown inside AgentMux's own Tower pane.
- **Panes.** Only renderers relate to panes. Each window (main, pool, tear-off) and each browser pane gets its own renderer. Terminal and agent panes have no CEF process of their own; their processes are the shells and agents srv starts, which Tower already lists. Labelling renderers with their window or pane is possible in Tower, not in the OS task manager (§7).
- **macOS.** The CEF helpers are already per-type named apps ("AgentMux Helper (GPU)", "… (Renderer)"), but they carry no icon. `agentmux-srv`, `agentmux-mcp` and `agentmux-bashwrap` are bare executables, shown by file name with a generic icon (§8.1).
- **Linux.** ELF binaries can't carry an icon; system monitors guess one. Names are fixable: srv's 15-character process name is currently the truncated, version-bearing `agentmux-srv-0.` (§8.2).

## 2. What Windows Task Manager reads

- **Icon:** the first icon group in the exe's resource section; none means the generic application icon. Copying or renaming keeps it, so packaging's `agentmux-srv` → `agentmux-srv-<ver>-windows.x64.exe` rename doesn't matter.
- **Name (Processes tab):** `FileDescription` from the `VERSIONINFO` resource; none means the file name. It is **one string per exe file**: every process running that file shows the same name.
- **Details tab, Properties, jump list header:** `FileDescription`, `ProductName`, `CompanyName`. The Details tab can also show a **Command line** column.
- **Grouping:** Task Manager groups an app with the processes it started, under the process that owns the window, and lists the app's windows by title when expanded.

## 3. Windows measurements

From the running processes (`Win32_Process` for parent PIDs and command lines; `ExtractIconEx(path, -1, …)` for the icon count; `VersionInfo` for the strings). Every copy of each binary agreed, across all six builds:

| Executable | Icons | FileDescription | CompanyName | Resources come from |
|---|---|---|---|---|
| `agentmux.exe` (launcher) | 1 | AgentMux v0.59.17 | AgentMux | `winres`, `crates/launcher/build.rs` |
| `agentmux-<ver>.exe` (CEF host; `agentmux-cef.exe` in dev) | 1 | AgentMux | AgentMux Corp | rcedit after the build, `scripts/inject-exe-icon.sh` |
| `agentmux-srv-<ver>-windows.x64.exe` | **0** | AgentMux Server v0.59.17 | AgentMux | `winres`, `crates/srv/build.rs`: strings only, **no `set_icon`** |
| `agentmux-mcp.exe` | **0** | *(empty)* | *(empty)* | **nothing**: no `build.rs` |
| `agentmux-bashwrap.exe` | **0** | *(empty)* | *(empty)* | **nothing**: no `build.rs` |

Side observations:

- **CompanyName is inconsistent:** `AgentMux` from `winres`, `AgentMux Corp` from `inject-exe-icon.sh` (and `package.json`, and every crate's `authors`).
- **The host's strings don't come from its `build.rs`.** The host exe is CEF's `bootstrap.exe` renamed (sandbox host, #1633), so what ships is what `inject-exe-icon.sh` stamps afterwards. The `winres` block in `crates/cef/build.rs` only matters for a non-sandbox raw build.

**What the 38 "AgentMux" rows were** (six instances running):

| Per instance | Count | Relates to |
|---|---|---|
| main (browser) process | 1 | the instance |
| `gpu-process` | 1 | the instance |
| utility: `network.mojom.NetworkService`, `storage.mojom.StorageService` | 1 each | the instance |
| utility: `audio.mojom.AudioService`, `video` | 0–1 each, on demand | the instance |
| `renderer` | 1–3 | one per window (main, pool, tear-off) or browser pane |

The main processes' window titles already name the instance: `59.17 - Tab 1 - AgentMux`, `Korp - tab1 - AgentMux`.

### 3.1 Everything else that shows up

The portable's `runtime/` holds exactly seven executables: the five above plus `tools/bin/jq.exe` and `tools/bin/rg.exe`.

| What | Shows as | Ours to fix? |
|---|---|---|
| `agentmux-srv`, `agentmux-mcp`, `agentmux-bashwrap` | generic icon | **yes**: §5 |
| CEF subprocesses | AgentMux icon already (they run the host exe), all named "AgentMux" | names: §6 |
| `muxlog`, `muxsh`, `muxspect` | **Node.js**: they're `.mjs` scripts (`crates/srv/src/backend/shellintegration/`) run by the user's `node.exe` | not without shipping our own executable for them; not proposed |
| `jq.exe`, `rg.exe` (bundled) | bare file name, generic icon (upstream ships none) | **no**: third-party; restamping would mislabel someone else's program and break their published hashes |
| Agents' processes (`claude.exe`, `codex`, `bash.exe`, `pwsh.exe`, `conhost`) | their vendors' icons | no |
| Installer and uninstaller | AgentMux icon (`packaging/windows/agentmux.iss`) | no change needed |
| `agentmux-remote` | not shipped in the Windows portable (runs on SSH hosts) | no |

## 4. Why Chrome looks different on Windows

1. **Icons.** Every Chrome process is `chrome.exe`, so all carry Chrome's icon. AgentMux ships several binaries; three carry no icon (§5).
2. **Grouping.** Everything in Chrome descends from the windowed browser process, so it collapses into one "Google Chrome (N)" entry. AgentMux's windowed process is the host, and its CEF subprocesses group under it. But the launcher starts both the host and `agentmux-srv`, so srv is the host's sibling and lists separately under Background processes, as does the launcher itself. `mcp` and `bashwrap` are started by the agents (`claude.exe`, `bash.exe`), so they never group under AgentMux. Re-parenting srv is a decision about its lifetime, not a cosmetic one; not proposed.
3. **Efficiency mode (the leaf).** Chrome opts its background renderers into EcoQoS itself. AgentMux uses EcoQoS nowhere; the only priority control is `BELOW_NORMAL_PRIORITY_CLASS` for agents' process trees (`agent:belownormalpriority`) and the CLI installer. See §11.

## 5. Windows icons: the proposed change

Embed the icon and version strings at compile time with `winres`, in each binary's own `build.rs`, as the launcher already does. Keep rcedit only for the host, the one exe `winres` can't reach.

**`crates/srv/build.rs`:** add the icon to the existing block:

```rust
let icon = std::path::Path::new("../cef/resources/win/agentmux.ico");
if icon.exists() {
    res.set_icon(icon.to_str().unwrap());
}
```

**`crates/mcp` and `crates/bashwrap`:** add a `build.rs` with the same block (strings + icon) and the build-dependency `srv` uses:

```toml
[target.'cfg(windows)'.build-dependencies]
winres = "0.1"
```

All three should emit `cargo:rerun-if-changed=build.rs` and `cargo:rerun-if-changed=../cef/resources/win/agentmux.ico`. Unlike the host's, these build scripts embed no git hash, so narrowing Cargo's rerun default is safe, and an icon change then actually re-embeds.

| Binary | FileDescription (proposed) | Today |
|---|---|---|
| `agentmux-srv` | AgentMux Server v<ver> | same |
| `agentmux-mcp` | AgentMux MCP v<ver> | *(file name)* |
| `agentmux-bashwrap` | AgentMux Shell Wrapper v<ver> | *(file name)* |

One `CompanyName` everywhere, `winres` crates and `inject-exe-icon.sh` alike: `AgentMux Corp`.

**Cost:** a ~23 KB icon in each of three binaries; no runtime cost (`bashwrap`, one process per shell command, doesn't touch its resource section to run). `winres` uses the Windows SDK's `rc.exe`, which Windows builds already need. `cfg(windows)` keeps macOS/Linux builds unaffected.

**Rejected:** running `inject-exe-icon.sh` on these three in `package-portable.sh`. It misses `cargo build`/`task dev`, rewrites finished PEs (with that script's Defender-lock retries), and hardcodes `FileDescription = "AgentMux"`, so every helper would read "AgentMux" — the very ambiguity §6 is about.

## 6. Windows names: the many "AgentMux" rows

**The constraint.** Task Manager's name is the exe file's `FileDescription`. Every CEF subprocess runs the host exe, so every one of them is "AgentMux", and no API lets a running process change the name Task Manager shows. Chrome has the same limit (its rows all read "Google Chrome").

What can be done, cheapest first:

1. **Put the build in the host's name.** Stamp `FileDescription = "AgentMux <ver>"` (and the channel for local builds, e.g. `AgentMux 0.59.17 (local)`) in `inject-exe-icon.sh`, which already writes that field. Every process of an instance then carries its build, so six instances side by side stop looking identical. A one-line change plus passing the version to the script.
2. **Mark each subprocess's command line.** CEF calls `OnBeforeChildProcessLaunch` with each child's command line before starting it (the binding exists in our `cef` crate; AgentMux doesn't implement it yet). Appending e.g. `--agentmux-instance=<channel>` makes the Details tab's Command line column say which instance a GPU/utility/renderer process belongs to, next to the `--type=` Chromium already puts there. It can't say which pane: a renderer is started before it's known which window or pane it will serve, and Chromium may reuse one.
3. **Split the subprocesses into their own exe.** CEF can launch subprocesses from a separate helper exe (`browser_subprocess_path`), which could be named "AgentMux Helper". This only splits main-vs-helpers (one path for every subprocess type, so not GPU-vs-renderer), and it collides with the sandbox design, where the host *is* CEF's bootstrap exe. Not recommended unless the sandbox work says it's compatible.
4. **Use AgentMux's own view.** Per-type and per-pane labels belong in Tower (§7).

Meanwhile, with no code change: Task Manager's Details tab → right-click a column header → *Select columns* → **Command line** shows `--type=renderer`, `--type=gpu-process`, `--utility-sub-type=network.mojom.NetworkService` on each row.

## 7. Do the processes relate to panes, and can they be labelled?

**What maps to what** (from the measurements and `crates/cef/src/app/mod.rs`, which notes "main window, every pool window, every tear-off window, and every browser-pane gets its own renderer process"):

| Process | Relates to |
|---|---|
| renderer | one window (main, hidden pool, tear-off) or one browser pane |
| GPU, network, storage, audio, video | the whole instance |
| terminal and agent panes | no CEF process: their content is drawn by the window's renderer, and their *processes* are the shell/agent srv starts (`bash.exe`, `claude.exe`, …), already listed per task in Tower |

**Labelling.** The OS task manager can't carry a per-pane label on any platform worth relying on (Windows: §6; macOS: helper names are per type at most; Linux: §8.2 allows a 15-character name per process but not a live pane title). Tower can:

- Each renderer reports its PID to the main process when a browser is created in it (the render-process handler's browser-created callback sends a process message carrying `std::process::id()`). The main process already knows which window or pane each browser belongs to, so it can map PID → label.
- Tower's AgentMux row then expands into labelled rows: `Renderer · window "Tab 1"`, `Renderer · browser pane github.com`, `Renderer · pool (idle)`, `GPU`, `Network service`, `Storage service`, each with its CPU and memory. That answers Tower's own open question about splitting out the CEF helpers.
- Caveat: Chromium can put two same-site browsers in one renderer, so a renderer row may list more than one pane.

## 8. macOS and Linux

### 8.1 macOS

Activity Monitor shows an app bundle's name and icon (`CFBundleName`, `CFBundleIconFile`) for a process whose executable is a bundle's main executable; a bare executable shows its file name and a generic icon.

| Process | Today (from `scripts/package-macos.sh`) | Proposed |
|---|---|---|
| Main app (`AgentMux.app`, launcher/host) | name and icon (`CFBundleIconFile` = `AgentMux`, `AgentMux.icns`) | — |
| CEF helpers: `AgentMux Helper`, `(GPU)`, `(Renderer)`, `(Plugin)`, `(Alloy)` in `Contents/Frameworks/` | **named per type already**, but each helper's `Info.plist` has **no `CFBundleIconFile`** and its bundle no `.icns` | copy `AgentMux.icns` into each helper's `Contents/Resources/` and add `CFBundleIconFile`; small, in the same loop that writes the plist |
| `agentmux-srv-<ver>-darwin.<arch>` in `Contents/MacOS/` | bare executable: versioned file name, generic icon | wrap in a nested helper app, e.g. `Contents/Helpers/AgentMux Server.app` (`LSUIElement`, icon), and teach `sidecar::resolve_backend_binary` the new path |
| `agentmux-mcp`, `agentmux-bashwrap` in `Contents/MacOS/tools/bin/` | bare executables | same treatment, keeping `tools/bin/` entries as symlinks into the helper apps so the PATH that agents use is unchanged |

The nested apps change signed bundle layout, so they go through codesign/notarization; the helper-icon fix doesn't move any executable. Verify the symlink approach on a real Mac: Activity Monitor should attribute a process to the bundle its resolved executable lives in.

### 8.2 Linux

ELF binaries have no resource section, so there's nothing to embed. Desktop system monitors guess an icon: GNOME System Monitor, for example, uses the app's window and desktop entry when there is one, then looks for an icon-theme icon named like the process, then falls back to a generic one. AgentMux's main window is matched today through `StartupWMClass` and the hicolor `agentmux` icon (`build-appimage-linux.sh`, `assets/linux/agentmux.desktop`); background processes get the generic icon.

Names, though, are per process on Linux, unlike Windows:

- The kernel's process name (`comm`, what `top`, `ps -o comm` and most monitors show) is the first **15** characters of the file name. srv's file is `agentmux-srv-<ver>-linux.x64`, so it shows as `agentmux-srv-0.` and changes with every version. `agentmux-bashwrap` shows as `agentmux-bashwr`.
- A process can set its own `comm` with `prctl(PR_SET_NAME)`. srv now sets `agentmux-srv`, and its document-parsing child `agentmux-srvdoc` (#4596). `agentmux-mcp` already fits in 15 bytes; `agentmux-bashwr` is left as the kernel cuts it.
- **Dropped:** naming each CEF subprocess by its `--type`. Chromium forks renderers and most utility processes from its zygote, so our entry point never runs in them: they would inherit the zygote's name, which is wrong rather than merely unhelpful. Tower labels them instead (§7).
- Icons: once names are stable, the `.deb`/`.rpm` can install hicolor icons under those names (`agentmux-srv` …) so monitors that look icons up by process name find them; an AppImage can only do that through its desktop-integration step. Which monitors honour it (GNOME System Monitor, KDE System Monitor, others) needs checking on real desktops before promising it.

## 9. Proposed order of work

1. **Windows icons** (§5): srv, mcp, bashwrap via `winres`; one CompanyName. Small, self-contained.
2. **Windows host name with the build** (§6.1) and the **macOS helper icons** (§8.1, first row). Small packaging changes.
3. **Linux process names** via `prctl` (§8.2). Small; makes srv's name stable. (The CEF per-type split was dropped, §8.2.)
4. **Tower: labelled CEF rows** (§7). A feature in the existing Tower pane; answers "which pane is this process".
5. **Command-line marker for the Details tab** (§6.2). Optional once Tower shows the same.
6. **macOS nested helper apps** for srv/mcp/bashwrap, and Linux theme icons by process name (§8). Larger (bundle layout, signing; per-desktop verification).

**Not built, and why.** Item 5: Tower now names every AgentMux process, the window or pane included, so a marker on the Details tab's command line would repeat it less usefully. Item 6: moving srv, mcp and bashwrap into nested `.app` bundles changes the signed bundle layout and how srv and the agents' PATH find those binaries, and it can only be verified on a Mac (Activity Monitor's attribution through a symlink in particular); theme icons by process name need checking per Linux desktop before they can be promised. Both are worth doing as their own change, verified on those platforms.

## 10. How to verify

**Windows**, after a `task package`:

```powershell
Add-Type -Namespace W -Name I -MemberDefinition '[DllImport("shell32.dll", CharSet=CharSet.Unicode)] public static extern uint ExtractIconEx(string f, int i, IntPtr[] l, IntPtr[] s, uint n);'
Get-ChildItem <portable>\runtime -Recurse -Filter 'agentmux*.exe' | ForEach-Object {
  '{0,-40} icons={1} desc="{2}" company="{3}"' -f $_.Name, [W.I]::ExtractIconEx($_.FullName, -1, $null, $null, 0),
    $_.VersionInfo.FileDescription, $_.VersionInfo.CompanyName }
```

Every line shows `icons=1`, a non-empty description and the same company; then check Task Manager with an agent running.

**macOS:** `plutil -p "AgentMux.app/Contents/Frameworks/AgentMux Helper (GPU).app/Contents/Info.plist" | grep CFBundleIconFile`, then Activity Monitor with the app open. Not measured for this report: no Mac was used.

**Linux:** `ps -eo pid,comm | grep -i agentmux` shows the stable names; then the desktop's system monitor. Not measured for this report.

## 11. Open questions

1. **Version in the name?** §5 and §6.1 put the build in `FileDescription` because several builds often run side by side (six did while measuring). Alternative: drop it everywhere and rely on the Details tab.
2. **One icon, or variants per role?** §5 uses the one brain icon, as Chrome does. A badge per role (server, MCP, shell) would tell them apart at a glance, at the cost of more icon files on three platforms.
3. **EcoQoS for hidden CEF renderers (pool windows)?** Chrome does this for background tabs. Separate spec if wanted: which processes, when to set and clear it, and measuring that it doesn't slow pane switches. Not for srv or agents, which the user waits on.
