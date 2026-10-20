# Spec: the Toolchain widget shows its state at once and settles quickly

**Date:** 2026-10-13
**Status:** proposed
**Plan item:** #10 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.

## 1. The problem

In the lab's widget benchmark, the Toolchain widget's content kept changing for about **3.6 s** after opening, even in a pre-warmed window, and 2–3 s as a floating pane. Every other widget settled in 0.2–0.6 s.

On mount, `frontend/app/view/toolchain/toolchain-view.tsx` (L228-252) starts all of this at once:

| Work | Count | srv cost |
|---|---|---|
| `ResolveCliCommand` | 6 core tools + 8 local models + ~11 agent CLIs (`toolchain-catalog.ts:109,252`, `providers/catalog.ts`) | each runs `where <cmd>`; if found, `<cli> --version` with a 5 s timeout (`crates/srv/src/server/cli_handlers.rs:903-922, 1010-1031`) |
| `checkLatestVersions` | 1 RPC → ~11 HTTPS GETs to the npm registry (`cli_handlers.rs:686-724`) | network; a new HTTP client per call |
| Widget probes (`probeWidget`) | 10 widgets (`providers/widget-catalog.ts`) | `ResolveCli`, **then, one after the other,** `widget.health`: an HTTP check of `127.0.0.1:<port>` with a 3 s timeout (`cli_handlers.rs:545-591`) |
| Docker | `ContainerRuntimeAvailableCommand`, then a re-check every 4 s (`frontend/app/store/toolchain-capabilities.ts:170-176`) | pings the daemon |

**Nothing is cached in srv:** every open starts every process again. The frontend's caches (`latestCache`, the capability store) are module-level, so each new window or floating pane starts empty.

**What probably dominates** (from the code; confirm with a trace, §4):
1. **About 25 `--version` launches at once.** Most agent CLIs are Node, often behind a `.cmd` wrapper. Each costs 0.3–1.5 s alone and more in parallel. For CLIs AgentMux installed itself (`source: "local_install"`) the version is already known from the install folder (`cli_handlers.rs:61-89`).
2. **Widget health after resolve, in series.** Most widgets aren't running, and on Windows a connect to a closed localhost port commonly takes 1–2 s while the OS retries.
3. **The npm "latest" results arrive late** and rewrite every agent-CLI row.
4. **The Docker row never settles.** Each 4 s re-check sets it back to "Checking…" (`toolchain-capabilities.ts:145`; the view's sync effect L215-224).

## 2. The change

1. **srv keeps a toolchain snapshot.**
   - A `toolchain.status` RPC returns every row's last known state at once: path, version, health, latest.
   - srv refreshes in the background with at most 4 probes at a time, and pushes changes as events.
   - The snapshot lives for srv's lifetime, so a second window, a floating pane or a reopen shows it instantly.
   - First use in a session still probes; later opens show the snapshot and refresh behind it.
2. **Cheaper probes:**
   - **`local_install` CLIs:** use the pinned version; no `--version` run.
   - **`get_cli_version`** is cached by path, mtime and size, so an unchanged binary isn't run again.
   - **Widget health** runs alongside the CLI probe, not after it, with a 300 ms connect timeout on 127.0.0.1 (a local server answers in milliseconds).
   - **npm "latest"** is cached in srv with a 6 h TTL and one shared HTTP client.
3. **Background re-checks don't show "Checking…".**
   - Docker's 4 s poll keeps showing the last state, and changes the row only when the state changes.
   - "Checking…" appears on first load and after a manual Refresh.
4. **Rows don't move.** Status pills reserve their width, so a result arriving doesn't shift the row.

**Not in this change:** the Toolchain view's layout and catalog content.

## 3. Tests

- **srv:**
  - `toolchain.status` returns the snapshot without starting a process when it's fresh;
  - a `local_install` CLI's version comes from its folder;
  - the version cache skips an unchanged binary;
  - the npm cache honours its TTL.
- **Frontend:**
  - the view renders the snapshot on mount;
  - a background capability re-check doesn't set "Checking…";
  - a row's layout doesn't change when its status arrives (fixed pill width).
- **Lab:** the widget benchmark (`memlab/winbench.mjs`, the method in `SPEC_WINDOW_OPEN_LATENCY_AND_WARM_POOLS_2026_10_13.md` §1.1). Expect Toolchain's settle time under 600 ms from a warm window on the second open in a session, and under 1.5 s on the first.

## 4. Before changing anything

Trace one cold open (srv's tracing plus the frontend's RPC timings) to confirm the cost split in §1. If the widget health checks dominate, change 2's timeout alone may bring the view under a second.
