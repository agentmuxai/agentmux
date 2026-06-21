# Retro: AgentMux leaks a Windows Terminal window per agent spawn (Win11)

**Date:** 2026-06-21
**Severity:** High (UX + resource leak; contributed to a wedged, unrecoverable session)
**Area:** `agentmux-srv` process spawning (Windows)
**Fix branch:** `agentc/fix-win-console-window-leak`
**Status:** Root-caused, fixed, verified

---

## 1. What the user saw

> "We got into a strange state with AgentMux. There are a bunch of open terminals
> ... when opening .47 from the desktop, it just says 'CEF' in the taskbar and
> doesn't load anything. I think things regarding lifecycle got corrupted."

Three symptoms, one underlying degraded state:
1. **~2 dozen (actually 53) terminal windows** open on the desktop.
2. A **v0.47.0** portable window stuck showing **"CEF"** in the taskbar, blank.
3. General "lifecycle corruption" — two instances running, orphaned processes.

---

## 2. Inventory (evidence)

### 2.1 Two instances + orphaned processes

`Get-CimInstance Win32_Process` (parent-linked) showed **two** full instances live
at once:

| Instance | Launcher | Host | Srv | Notes |
|----------|----------|------|-----|-------|
| 0.46.6 | `agentmux.exe` 508972 | 212332 (+8 render kids) | 321524 | **+3 orphans** 202972/401192/412208 and conhost 225472 whose parent **PID 556136 was dead** |
| 0.47.0 | `agentmux.exe` 153864 | 178204 (+10 render kids) | 375764 | window "Window 3 - tab1 - AgentMux" |

The dead-parent orphans (556136) are the literal "lifecycle corruption": a host
that died without reaping its children, leaving zombies behind.

### 2.2 The "terminals" are Windows Terminal windows

A `user32!EnumWindows` sweep filtering console/terminal window classes returned **54
visible windows**:

```
VISIBLE CONSOLE/TERMINAL WINDOWS: 54
  53 × PID 407772  WindowsTerminal  class=CASCADIA_HOSTING_WINDOW_CLASS  title="Terminal"
   1 × PID 313696  pwsh             class=ConsoleWindowClass             (this admin session)
```

**53 of them belong to a single `WindowsTerminal.exe` (PID 407772)**, whose parent
is `svchost.exe` — i.e. the Windows 11 **DefTerm** (default-terminal) handler.
Started `06/20 09:56`, it had been accumulating "Terminal" windows for ~24h.

### 2.3 The "CEF" window

`.47` host log (`channels/local-main-b28b7a-86f99d43/.../agentmux-host-v0.47.0.log`)
showed the stuck window healthy up to:

```
[frontend] [initApp] pool mode — deferring init until pool:promote or pool:new-window
```

i.e. a **pre-warmed window-pool window that never received its promote event** — so
it stays blank with the default CEF title. Memory was tight at the time
(`load_pct 84`, `avail_phys 9.8 GB`) with two instances + orphans competing. This is
a *consequence* of the degraded state, not the root cause (§6).

---

## 3. Root cause

### 3.1 The Windows 11 mechanism

`agentmux-srv` runs **without a console of its own**. On Windows, when such a process
launches a **console** child *without* the `CREATE_NO_WINDOW` (`0x0800_0000`) creation
flag, the OS allocates a fresh console and hands it to the user's **default terminal
application**. On Windows 11 that is **Windows Terminal**, so each such spawn becomes a
**new Windows Terminal window**. ConPTY/`portable-pty` spawns (terminal panes,
`shell.rs`) are headless and exempt — the leak is only from plain `Command` spawns.

### 3.2 The gap: agent spawn paths omit the flag

A full audit of `Command` spawn sites in `agentmux-srv` shows the flag is applied in
most places but **missing in three** — including the highest-frequency one:

| Spawn site | Spawns | `CREATE_NO_WINDOW`? |
|---|---|---|
| `blockcontroller/acp.rs` | ACP agent CLI | ✅ set |
| `blockcontroller/subprocess.rs` | subprocess agent CLI | ✅ set |
| `blockcontroller/shell.rs` | terminal pane (ConPTY) | ✅ set (+ headless PTY) |
| `backend/shell_node.rs`, `tool_store.rs`, `server/cli_handlers.rs` | helpers/probes | ✅ set |
| **`blockcontroller/persistent.rs`** | **persistent agent CLI (the main agent type)** | ❌ **missing** |
| **`agents/runner.rs`** | one-shot `claude --print` task/drone agents | ❌ **missing** |
| **`backend/lsp/supervisor.rs`** | LSP servers (ts-language-server, pyright, gopls, rust-analyzer) | ❌ **missing** |

`persistent.rs` is the dominant leak: it spawns the agent CLI (`claude.cmd`) for
**every persistent agent**, and **once per start / resume / respawn**. The
`core.rs:34` doc comment even names `CREATE_NO_WINDOW` ("must come after the env
setup") — the intent was known, but the flag was never actually applied on this path.

### 3.3 Why it ran away to 53

The per-spawn leak is multiplied by lifecycle churn:
- **Health-watchdog flapping.** As documented on 2026-06-18 (Naki), a busy-resuming
  agent is mislabeled `Healthy→Stalled→Dead` and **respawned** — each respawn is a new
  leaked window.
- **Two instances** running for ~24h, each restarting agents.
- **Orphaned hosts** that died without cleanup.

One leaked window per spawn × many spawns × a day = 53.

---

## 4. The fix

Add the Windows-only flag to the three spawn builders, after `apply_working_dir`
(per the `core.rs:34` ordering note), copying the existing `acp.rs`/`subprocess.rs`
idiom. All three use `tokio::process::Command`, whose `creation_flags` is inherent
(no import), and all three already pipe stdio — so the console was never needed.

```rust
#[cfg(windows)]
{
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
```

Files changed:
- `agentmux-srv/src/backend/blockcontroller/persistent.rs`
- `agentmux-srv/src/agents/runner.rs`
- `agentmux-srv/src/backend/lsp/supervisor.rs`

---

## 5. Verification (evidence)

**Cleanup performed during diagnosis (proves the source of the windows):**
- Killed both instances (launchers first, so the Job Objects reaped children) →
  `agentmux procs: 0`.
- Killed `WindowsTerminal.exe` 407772 → visible console/terminal windows dropped
  **54 → 1** (only this admin pwsh remained). Confirms all 53 were the leak, and that
  no live agent owned them (they were already detached).

**Build / test (the fix):**
- `cargo check -p agentmux-srv`: **clean** (exit 0, 27s; 67 warnings, all pre-existing dead-code — none from the change).
- `cargo test -p agentmux-srv --bins`: **1221 passed, 0 failed, 2 ignored** (compiles + green).

**Behavioural expectation:** with the flag set, a persistent-agent start/resume spawns
the CLI with stdio piped and **no console allocation**, so DefTerm never opens a
Windows Terminal window. ConPTY terminal panes are unaffected (already headless).

---

## 6. Follow-ups (separate from this fix)

1. **Health-watchdog false "Dead" on resume** (2026-06-18 retro). It mislabels a
   busy-resuming agent as dead and forces respawns — which, pre-fix, each leaked a
   window. Distinguish "process alive, no first token yet" from "hung/dead".
2. **Window-pool promotion wedging** (the "CEF" blank window). The pool window never
   got its `pool:promote`. Investigate whether promotion can stall under memory
   pressure / many windows, and add a fallback so a forwarded `open_new_window`
   always yields a usable window or a clear error rather than a blank "CEF" frame.
3. **Orphan reaping.** A host that dies should not leave child render processes +
   conhosts behind (PID 556136 case). Verify the Job Object kill-on-close path covers
   abnormal host exit, not just launcher-initiated shutdown.

---

## 7. Timeline

- **2026-06-20 09:56** — `WindowsTerminal.exe` 407772 started by DefTerm; window
  accumulation begins.
- **2026-06-21 ~09:2x–09:34** — user opens v0.47.0; pool window wedges ("CEF"); two
  instances + orphans; 53 terminal windows visible.
- **2026-06-21** — diagnosis: inventory → EnumWindows → spawn audit; killed instances
  + WindowsTerminal (54→1 windows); root-caused to missing `CREATE_NO_WINDOW` in
  `persistent.rs`/`runner.rs`/`supervisor.rs`; fix applied + verified; PR opened.
