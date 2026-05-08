# Browser-pane reducer migration — diagnostic-first roadmap

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-08
**Replaces (in spirit):** the discarded PR #737. The catalog at [`browser-pane-state-catalog.md`](./browser-pane-state-catalog.md) (commit `5af7474f` in git history) is the precondition input.

## Why a roadmap?

The previous attempt (PR #737) tried to migrate the browser pane to a reducer + slot-lifecycle pattern in **one step** while also adding two product features (live title, live favicon). It introduced regressions in the address-bar typing path and first-load timing — both subtle, both untraceable without instrumentation. PR was closed; main is back at the last working state (#736, `eaf591d9`).

Rule for the next attempt: **observe first, refactor second.** No structural change ships without diagnostic logs that can characterize the *current* behavior and prove the migrated behavior matches it.

The roadmap below is **strict sequence**. Each phase is its own PR. Each merge is gated on logs from the prior phase confirming the change is observable and benign.

---

## Phase 0 — Catalog (DONE)

Inventory of every state cell + read/write call site + focus interaction.

**Status:** written, lives at `docs/specs/browser-pane-state-catalog.md` (or git history at `5af7474f` if not yet restored to main).

**Action:** cherry-pick that single commit into a doc-only PR before starting Phase 1, so the catalog is the working reference.

---

## Phase 1 — Diagnostic instrumentation (no behavior change)

Goal: every state write, every IPC event, every reactive read of the URL/title/favicon path, and every focus transition emits a log line we can grep.

### Logs to add

All routed through `console.log` (which goes through `fe_log_structured` → host log per `CLAUDE.md`'s "Console Logs Go to the Backend" note). Tag every line with `[browser-pane:diag]` + the blockId prefix so they're greppable.

#### In `BrowserViewModel`

| Event | Log line |
|---|---|
| Constructor enter | `[browser-pane:diag][${blockId.slice(0,7)}] ctor meta.url=${meta.url}` |
| listenEvent registered (each of 3) | `[browser-pane:diag][${b}] sub-registered name=${eventName}` |
| nav-state event in | `[browser-pane:diag][${b}] nav-state recv url=${u} url_only=${o} can_back=${b} can_forward=${f}` |
| title-change event in | `[browser-pane:diag][${b}] title-change recv title=${t}` |
| clicked event in | `[browser-pane:diag][${b}] clicked recv` |
| `navigate(url)` | `[browser-pane:diag][${b}] navigate(url=${u}) caller=${pc?}` |
| `goBack/Forward/Reload` | `[browser-pane:diag][${b}] goBack/goForward/reload` |
| `setUrl/setTitle/setFavicon/setLoading/setCanGoBack/setCanGoForward/setError` | `[browser-pane:diag][${b}] state-write key=${k} value=${v}` |
| `dispose` | `[browser-pane:diag][${b}] dispose` |
| Late event after dispose | `[browser-pane:diag][${b}] post-close-event-dropped name=${n}` |

#### In `browser-view.tsx`

| Event | Log line |
|---|---|
| Component mount | `[browser-pane:diag][${b}] view-mount initial-addressBar=${a}` |
| `createEffect` fires (urlAtom→addressBar sync) | `[browser-pane:diag][${b}] sync urlAtom=${u} addressBar=${a} willUpdate=${b}` |
| URL bar `<input>` keydown | `[browser-pane:diag][${b}] input-keydown key=${k}` (rate-limited / sampled if too noisy) |
| Address bar submit (Enter) | `[browser-pane:diag][${b}] input-submit value=${v}` |
| Address bar focus / blur | `[browser-pane:diag][${b}] input-focus` / `input-blur` |
| `paneCreated` flip | `[browser-pane:diag][${b}] paneCreated=true` |
| Component unmount | `[browser-pane:diag][${b}] view-unmount` |

#### In `agentmux-cef` (host)

| Event | Log line |
|---|---|
| `on_after_created_browser_pane` | already exists at info level |
| `on_load_end_browser_pane` enter | already exists |
| `browser-pane-nav-state` emit | `[browser-pane:diag][${block_id[..7]}] emit-nav-state url=${u} url_only=${o}` |
| `browser-pane-clicked` emit | `[browser-pane:diag][${block_id[..7]}] emit-clicked` |
| Future: title-change emit | `[browser-pane:diag][${b}] emit-title-change title=${t}` |

### Acceptance for Phase 1

- A user can open `muxlog host '\[browser-pane:diag\]'` and see, for a single navigation, the full sequence of: host emit → renderer subscription receive → state writes → memo/effect propagation → DOM render. The sequence should match the current (pre-refactor) mental model.
- No behavioral change. State management is identical to today; logs are pure side-channel.

### Phase 1 PR

`agenta/browser-pane-diag-logs`. Tiny — only adds tracing calls. Reagent should land it quickly.

---

## Phase 2 — Behavioral characterization (no code change)

Use Phase 1's logs to capture **observed** behavior in three scenarios. Append findings to the catalog.

### Scenario A — Fresh pane open

Open new browser pane. Expected (per catalog):

1. Constructor runs → ctor log
2. 3 listenEvent regs → 3 sub-registered logs
3. `navigate(initialUrl)` → navigate log
4. `setUrl/setLoading/setError` → state-write logs
5. `RpcApi.SetMetaCommand({url})` (renderer) → backend log
6. Host: pane HWND created → on_after_created_browser_pane log
7. Host: page loads → on_load_end_browser_pane log
8. Host: emit nav-state → diag emit log
9. Renderer: nav-state recv → state-write logs (url, faviconUrl, canGoBack, canGoForward, loading=false)
10. Address bar `createEffect` syncs urlAtom→addressBar → sync log
11. Component renders with the new URL

### Scenario B — User types in address bar mid-navigation

Open pane. Wait for first load. Click address bar, start typing. While typing, click a link inside the page. Capture log.

Verify:
- `input-focus` log
- Multiple `input-keydown` (or sampled) logs
- nav-state recv log (link click triggered navigation)
- **Does the sync createEffect clobber the user's typing?** Look for `willUpdate=true` log when addressBar !== urlAtom AND the typed text is in addressBar.
- Document observed behavior — is this the bug we suspected?

### Scenario C — Dispose during in-flight navigation

Open pane. Submit a slow URL (e.g., `https://httpstat.us/200?sleep=5000`). Close the pane mid-load.

Verify:
- `dispose` log
- `post-close-event-dropped` log if late nav-state arrives
- No exceptions or stack traces

### Acceptance for Phase 2

A short retro doc (`docs/retro/browser-pane-observed-behavior-2026-05-08.md`) describing each scenario's actual log sequence. This is the **baseline** — any future refactor must produce the same sequence (modulo stable refactor markers).

### Phase 2 PR

Optional — could just be retros without code. If we add a few `data-testid` hooks for future automation that stays.

---

## Phase 3 — Pure reducer (one cell at a time)

Migrate state to a pure reducer **incrementally**. One cell per PR. Each PR keeps the existing call sites; only the internal storage changes.

### 3a — `closed` flag → reducer

The simplest cell with the fewest dependencies. State holds `{ closed: boolean }`. `dispose()` dispatches `Disposed`; everything that checks `this._closed` reads through the reducer.

Verify with Phase 1 logs: same sequence of dispose-related logs as today.

### 3b — `loading` + `error` → reducer

Mutually-exclusive pair. State adds `{ loading, error }`. `setLoading`/`setError` route through dispatch.

### 3c — `canGoBack` + `canGoForward` → reducer

Co-update on nav-state.

### 3d — `url` → reducer

This is the **danger cell**. Triple-check:
- `urlAtom` accessor must be IDENTICAL semantics for downstream consumers (the address bar's `createEffect`, the reload button, the placeholder gate).
- Add an extra log: `[browser-pane:diag][${b}] urlAtom-read consumer=${stack-frame}` for the duration of this PR (remove after merge) so we can verify no consumer changed semantics.

If Phase 1's log sequence diverges after 3d, **stop, revert, investigate**. This is the cell that broke last time.

### 3e — `title` + `faviconUrl` → reducer

Lightweight. Title fallback to "Browser" lives in the reducer.

### Each 3x PR

Tiny, single-cell, gated on Phase 1 log parity. Reagent + codex review per usual.

---

## Phase 4 — Slot lifecycle + audit trail

Only after every cell is reducer-backed AND Phase 1 logs continue to match.

- Move state ownership out of the model into `frontend/app/store/browser-pane-state-store.ts`.
- `registerPane(blockId, projections)` / `unregisterPane(blockId)` / `dispatch(blockId, cmd, source)`.
- `recordDispatch` audit per slice convention.

The previous attempt missed two preservation rules: the `addressBar` typing buffer stays component-local, and the `addressBar ↔ urlAtom` sync is reactive on `urlAtom` (any timing or reactivity change to `urlAtom` will leak through to typing). Phase 1 logs are the safety net that catches a re-occurrence.

---

## Phase 5 — Diagnostic panel surface

Wire `recordDispatch` for `browser-pane-state` into the existing diagnostics panel. At-a-glance view of every dispatched command per pane.

---

## Phase 6 — Title + favicon (the original feature)

Only NOW, after the reducer is solid and characterized, add the two product features that PR #737 tried to combine with the migration.

- Host: extend `on_title_change` to emit `browser-pane-title-change` for panes.
- Renderer: subscribe, dispatch `TitleChangeReceived` (already a reducer command after Phase 3e).
- Renderer: derive favicon from URL origin in the nav-state command (also already a reducer transition after Phase 3e).

This phase is a few lines now — the heavy lifting is in Phase 3.

---

## Stop conditions (don't ship the next phase if any holds)

1. Phase 1 logs show new lines or missing lines that don't match the prior phase's baseline.
2. Address bar typing test shows clobber that wasn't there in Phase 0 baseline.
3. First-load timing shifts by more than ~50 ms vs Phase 0 baseline (look at the gap between `ctor` and `paneCreated=true` logs).
4. Any unhandled exception in the diag logs.
5. Reagent / codex flag a regression that the diag logs confirm.

If a stop condition triggers: **revert that phase, investigate via logs, do not iterate.** The lesson from PR #737 is that "iterate on review feedback" doesn't work for cross-cutting state migrations — each iteration introduces new subtle interactions.

---

## Effort estimate

| Phase | PRs | LOC | Days |
|---|---|---|---|
| 0 — catalog (done) | 1 doc-only | 200 | 0 |
| 1 — diag logs | 1 | ~60 | 0.5 |
| 2 — characterize | 0–1 | ~30 | 1 |
| 3a-3e — reducer cells | 5 | ~300 | 3-5 |
| 4 — slot lifecycle | 1 | ~150 | 1 |
| 5 — diag panel | 1 | ~80 | 0.5 |
| 6 — title+favicon | 1 | ~60 | 0.5 |

Total: ~9 small PRs, ~6-9 calendar days. Compared to PR #737 (one giant attempt that failed): more PRs, more reviews, but each one ships verifiable behavior.

---

## Cross-references

- `docs/specs/browser-pane-state-catalog.md` — Phase 0 inventory
- `docs/specs/frontend-reducer-conventions-2026-05-03.md` — slot lifecycle / audit conventions to match in Phase 4
- `docs/specs/agent-pane-document-reducer-2026-05-03.md` — slice #1 reference impl
- `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md` — register browser-pane as slice #9 here when Phase 4 ships
- GitHub Discussion #707 — append progress notes per phase
- PR #737 (closed) — what not to do
