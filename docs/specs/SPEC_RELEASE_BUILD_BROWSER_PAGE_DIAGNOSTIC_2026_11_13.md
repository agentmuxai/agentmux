# Spec: `muxspect browsers`, the pages of a running release build with their CPU

Task #47543 (spec first, then the build). Loap-3@narko, 2026-11-13.

**Status:** active — approved 2026-11-20 (task #47543) with decisions D1 to D5 as in §5. Shipped: step 1, the host route `page_probe` (#7968, which adds this spec), and steps 2 to 4 (the srv join and `GET /api/v1/muxspect/browsers`, the `muxspect browsers` command, the runbook and `docs/MUXSPECT.md`; the PR after it). The scale line is one total for the instance's other processes, not a split by kind. Remains: step 5, a release-build proof that is a lab with its own OK.

## 1. The problem

The CPU-floor RCA ([RCA_IDLE_CPU_FLOOR_PREWARMED_POOL_WINDOWS_2026_10_21.md](../analysis/RCA_IDLE_CPU_FLOOR_PREWARMED_POOL_WINDOWS_2026_10_21.md) §6 item 5, §7.2, §7.3) lost most of its time on one question: *which page is this busy renderer drawing, and what is that page doing?* A release build has no DevTools server (#3681), so the answer needed a second instance started with `AGENTMUX_CDP_PORT`, a script to match `Performance.getMetrics` `ProcessTime` against `Get-Process` CPU, and a guess when two renderers looked alike. The runbook ([RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md](../debug/RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md) §4, §5) still tells the next agent to do that.

The task says the pid-to-page mapping is "the hard part" and that nothing exposes a pid. **That was true on 2026-10-21 and is not true now.** The pieces exist; none of them answers the question on its own:

| Piece | Where | What it gives | What it lacks |
|---|---|---|---|
| Renderer to browser map | `crates/cef/src/renderer_map.rs` (`snapshot`, `POST /agentmux/browser/renderer_map`) | Every renderer pid with its browser's label, pane block id, window id, pool flag, and one entry per cross-site iframe process. Built from the renderer's own report (`OnBrowserCreated`, `OnContextCreated`), so it is exact, not inferred | No page facts, no CPU |
| Process CPU and memory | `agentmux_procstats::ProcInfo` (`cpu_ns` cumulative, private bytes); sampled by srv's Tower | CPU for any pid | Does not know which page a pid serves (Tower uses the map only for a label) |
| In-process CDP | `crates/cef/src/browser_api/cdp.rs` (`CdpSession::attach(state, label)`, `call`) | Any CDP method on any browser by label, **with no DevTools server** (it uses `SendDevToolsMessage`) | Nothing calls it for a diagnostic |
| Memory timeline | `memory_heartbeat/renderer_memory_log.rs` | One host-log line per renderer every 5 min: pid, role, origin, private bytes | Memory only, history only, no visibility or animations |

So the build is a join of three things that already exist, plus one small probe. No CEF change, no new process message, no restart.

## 2. What the operator or an agent gets

One command, on any running instance, including a release build:

```
muxspect browsers [--sample <seconds>] [--json]
```

```
PID    CPU%   ROLE          PAGE                          VISIBLE  ANIMATIONS (running)         SHARED
5128   47.2   pool-window   AgentMux (pre-warmed window)  visible  2 css: startup-spin x2       -
9912   20.6   app-window    Tab 1 - AgentMux              visible  0                            -
7340    2.1   browser-pane  https://example.com           hidden   0                            -
6021    0.0   browser-pane  https://example.com (frame)   hidden   -                            pane b1f3 also
4 other AgentMux processes (GPU, network service, window host, srv, launcher): 14.7% together, for scale
```

CPU is percent of one core over `--sample` seconds (default 3), the unit the runbook already uses. `--json` returns the same rows for a script. Each row answers, without a restart: which page, which process, how busy, whether Chromium thinks it is visible, and what keeps it painting.

## 3. Design

### 3.1 Where each part lives

```
muxspect browsers  ->  srv  GET /api/v1/muxspect/browsers      (join, CPU, Linux pid translation)
                          |-- host  POST /agentmux/browser/page_probe  (new: the page facts)
                          |-- host  POST /agentmux/browser/renderer_map (exists: pid <-> label)
                          '-- procstats samples, twice, `sample` seconds apart (exists)
```

- **The host answers what only the host knows: the page.** `page_probe` is a new route in `crates/cef/src/browser_api/` next to `renderer_map`, authorised with the same bearer token. For each browser from `state.list_browsers()` it attaches an in-process CDP session by label and runs one fixed expression (§3.2). It does not take a script from the caller: this is a diagnostic, not a second `eval`.
- **srv does the join and the CPU.** It already samples processes for Tower, already asks the host for `renderer_map`, and already owns the Linux pid translation (`backend::tower::agentmux::own_view_pid`: the sandbox's PID namespace makes the renderer's own pid different from the host's). Doing the join in srv means one translation, one CPU source and one place the numbers come from.
- **The surface is `muxspect`**, as a new `browsers` command, not the Instance panel. Reasons: the people who ask this question are agents and engineers in a terminal who want text they can paste into an RCA and a `--json` they can script; muxspect already authenticates the same way every other `/api/v1/*` route does; it needs no UI, so no macOS/Linux look checks; and a panel would add a screen for a question asked a few times a month. The Instance panel stays an option for a later phase: it would call the same srv route (§6).

### 3.2 The probe

One expression run in each browser's main frame, returning a small object. Plain data, no DOM nodes, no page content beyond what is listed:

```js
(() => {
  const running = document.getAnimations().filter((a) => a.playState === "running");
  const kind = (a) => a.constructor.name;               // CSSAnimation, CSSTransition, Animation
  const name = (a) => a.animationName ?? a.transitionProperty ?? "";
  return {
    visibility: document.visibilityState,
    hasFocus: document.hasFocus(),
    animations: running.length,
    kinds: [...new Set(running.map((a) => kind(a) + (name(a) ? ":" + name(a) : "")))].slice(0, 8),
    title: document.title.slice(0, 120),
  };
})()
```

Two more reads, both CDP, no page script: `Performance.getMetrics` before and after the sample gives per-page main-thread seconds per second (`TaskDuration`, `ScriptDuration`, `LayoutDuration`, `RecalcStyleDuration`), which is what the runbook §5 asks for by hand and shows *what kind* of work a busy page does. It is the second column to look at after CPU%, shown under `--detail`.

Running the expression in the page's own JS world means a hostile page could return false data. That is acceptable for a diagnostic whose output is read by a person, and the output says it is the page's own answer; the pid, role and CPU columns do not come from the page. If this ever has to be trusted, the same expression runs in an isolated world (`Page.createIsolatedWorld`, then `Runtime.evaluate` with its `contextId`), which a page cannot patch. **Recommendation: use the isolated world from the start**, since it costs one more CDP call per browser and removes the question.

### 3.3 What it must not do

- **Never wake a sleeping page.** Korp's hibernation (`browser_api/hibernation.rs`) discards hidden panes under memory pressure; attaching CDP to one must not recreate it. A pane whose lifecycle is not `Live`, or that the hibernation state says is asleep, is listed with `PAGE: asleep` and no probe, and no CPU row (it has no renderer).
- **Never hang on a stuck page.** A renderer that is the very thing being diagnosed may be wedged in JS. Each probe has a 1.5 s timeout (the CDP client's own 10 s bound is for actions) and all browsers are probed in parallel. A timeout is a finding: the row says `no answer 1.5 s` with its pid and CPU, which is what a wedged renderer looks like.
- **Never collect what the page shows (AgentY, D2).** The host has two fixed expressions, chosen by what the browser is, so nothing a person's page says is ever read, not even to be dropped:
  - **App windows and pool windows** (AgentMux's own pages): the expression in §3.2, with `title` and the animation names (`AgentMux (pre-warmed window)`, `startup-spin` are the whole point).
  - **Browser panes:** `{ visibility: document.visibilityState, hasFocus: document.hasFocus(), animations: <running count> }` and nothing else. The row's page column is the origin from the browser's URL (`origin_of`, scheme, host and port, never a path or query), which the host already has.
- **A caller learns nothing about a pane that `Pane layout` would not already tell it (AgentY).** The srv route applies the same visibility the `Pane` tool's layout answer does: a caller sees the panes it could already see there, and an AgentMux window row is the same for everyone. The build's PR says what a container agent sees.
- **No `Runtime.enable` on a person's pane (AgentY, D3).** The isolated world is made with `Page.createIsolatedWorld` and read with `Runtime.evaluate` and its `contextId`, which needs no `Runtime.enable`; if a CDP version ever requires it, the pane is listed without a probe instead.
- **A Vault tab is probed with the pane expression only.** `/agentmux/browser/eval` refuses a tab that holds a Vault value or shows a secret once. The probe runs there too, because the pane expression returns no text and no title, so it cannot read the value; a test with a page that holds a Vault value proves the answer carries only visibility, focus and the running count.
- **No `eval` route reuse.** The probe has fixed expressions and takes nothing from the caller, so `/agentmux/browser/eval`'s guard for agent-supplied scripts does not apply; the two pane rules above are what stands in for it.
- **Read-only, no state.** Nothing is stored; no log line is added (a diagnostic that writes the thing it measures changes it). `Performance.enable` is turned off again after the read **and when the probe times out or fails** (AgentY, D4): the disable is sent from the cleanup path, not the success path.

### 3.4 The join (pure, unit-tested)

`fn browser_rows(entries: &[RendererEntry], probes: &[PageProbe], before: &[ProcInfo], after: &[ProcInfo], secs: f64) -> Vec<BrowserRow>`

- One row per renderer pid. A pid that serves several browsers (`renderer_map` already supports this) is one row with `SHARED` naming the others; its CPU is the process's and is not split between them (the existing `pane_memory` makes the same call for memory).
- A frame-only renderer (`main: false`, a cross-site iframe) is a row marked `(frame)` with the page of the browser it belongs to.
- CPU% = `(after.cpu_ns - before.cpu_ns) / secs / 1e9 * 100`; a pid present in only one sample (started or exited during the sample) gets no CPU and says so.
- Roles reuse the three names the memory timeline already logs (`app-window`, `browser-pane`, `pool-window`).
- Linux: entries carry the namespace-local pid; srv maps them to host pids with `own_view_pid`, as Tower does.
- A final line sums the AgentMux processes that serve no page (GPU, network service, host, srv, launcher), so the page rows have a scale.

### 3.5 The runbook

`docs/debug/RUNBOOK_ATTRIBUTE_AGENTMUX_CPU_ON_WINDOWS.md` §4 ("Which page a renderer draws") becomes: run `muxspect browsers`; the `AGENTMUX_CDP_PORT` second instance and `page-pids.mjs` move to a "when the running instance cannot be queried" appendix, with the existing text unchanged. §5's first two bullets (running animations, visibility) point at the `ANIMATIONS` and `VISIBLE` columns; the `Performance.getMetrics` bullet points at `--detail`. §2 (which processes) keeps `measure.ps1`: it works with no AgentMux code running and is the check that `muxspect` itself isn't what is using the CPU. `docs/MUXSPECT.md` gets the command.

## 4. Build plan (after approval)

| Step | Change | Check |
|---|---|---|
| 1 | `crates/cef/src/browser_api/page_probe.rs`: the route, the expression as a constant, the timeouts, the sleeping-pane skip, the pane redaction; route in `mod.rs` | unit tests for the redaction and the skip; the expression run in jsdom with a stubbed `getAnimations` |
| 2 | srv: `browser_rows`, the `muxspect/browsers` handler (host calls through the same helper Tower uses), the sample | unit tests for the join: shared renderer, frame-only renderer, pid in one sample only, no probe answer, Linux translated pid |
| 3 | `muxspect.mjs`: the `browsers` command, text and `--json`, help | a test of the command's text and `--json` output on a canned srv answer |
| 4 | Runbook and `docs/MUXSPECT.md` | none |
| 5 | A proof on a **release build** (not `task dev`, which has CDP on): `muxspect browsers` on an installed build lists every window and pane with its pid, CPU, visibility and running animations, and the pool windows read as the RCA describes (hidden, 0 animations on a build with #5145) | the one thing a release build can show that a dev build can't: it works with no `AGENTMUX_CDP_PORT` |

Step 5 is a measurement on the person's running instance, so it needs AgentY's relayed OK first (workspace rule on labs); steps 1 to 4 do not. Windows, macOS and Linux all build the same code (`renderer_map` and `cdp.rs` are not Windows-only); the `muxspect` text output has no layout to look-check. The CEF crates need a CEF build: on narko that means one build at a time and no release build.

## 5. Decisions for AgentY

| # | Question | Recommendation |
|---|---|---|
| D1 | Surface | **Answered: `muxspect browsers` first**; the Instance panel later on the same route |
| D2 | Pane privacy | **Answered: stricter than drafted.** A pane's probe never collects the title or animation names (second fixed expression); srv gives a caller nothing about a pane that `Pane layout` would not; the PR says what a container agent sees |
| D3 | Probe world | **Answered: isolated world, and no `Runtime.enable` on a person's pane** |
| D4 | `Performance.getMetrics` under `--detail` | **Answered: yes**, with `Performance.enable` turned off again even when the probe times out |
| D5 | Seat | **Answered: Loap-3 builds all of it.** Korp's seat owns `browser_api` and hibernation, so Korp reads step 1's diff before it merges |
| Hold | When | **Approved 2026-11-20; steps 1 to 4 are being built.** Step 5 stays a lab with its own OK |

## 6. Not in this spec

- A UI for it (the Instance panel, or a Tower pane column). The route is the same; the panel is a later task.
- Per-thread CPU. The RCA's thread work (§7.4, §7.5) needs OS calls on the process, not the page; `measure.ps1` stays the tool for it.
- Fixing any CPU finding it shows. The point is to find them in minutes.
- Cross-instance `muxspect browsers` (another AgentMux on the LAN). muxspect's cross-instance phases carry commands generically; this one joins them when they ship, with no change here.
- Changing what the renderer reports. `renderer_map` is complete for this purpose.
