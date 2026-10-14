# Spec: opening and closing windows must not leave renderers behind

**Date:** 2026-10-13
**Status:** active. §3.2 is confirmed and fixed (§4.2); the other changes are still open.
**Plan item:** #8 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Background:** `docs/retro/retro-window-lifecycle-leak-2026-07-04.md` (round 6, demote-into-pool) and `SPEC_PARK_AND_BLANK` (the blank fallback).

## 1. The problem

The lab ran about 60 cycles of "Open in New Window" and "Open in Floating Pane" from the widgets bar, each closed again (`window.api.closeWindow`), on an isolated production portable of `main` on Windows.

| | Before | After |
|---|---:|---:|
| AgentMux's CEF processes (private working set) | 189 MB | **1.47 GB** |
| Renderer processes | 3 | 12 |

The 12 renderers were:
- **7 "AgentMux (pre-warmed window)" pages at 115–124 MB each,** each with about 13 documents (CDP `Memory.getDOMCounters`) and a 30–34 MB JS heap. A fresh pool window is 52–58 MB with 2 documents.
- one `about:blank` page;
- the main window, two fresh pool windows and the pane-pool window.

The host log showed:
- "round 6: promoted window demoted back into pool" 41 times;
- "[pool] eviction: on_before_close never came — unregistered the pool window directly" several times.

## 2. How windows close today

All closes enter `close_window` (`crates/cef/src/commands/window/lifecycle.rs:132-174`) → `CloseWindowTask` (`ui_tasks/window.rs`).

| Window | On close | Its renderer |
|---|---|---|
| Promoted pool window (`window-pool-*`) | **Demoted**: server cleanup (`demote_srv_cleanup`), hidden off-screen, `frame.load_url(pool URL)`, back into `pool.unpromoted` (`window_pool.rs:939-1066`) | reused |
| Cold top-level window (`window-{uuid}`) | **Adopted** into the pool, same as above (`ui_tasks/window.rs:258-277`) | reused |
| Either, when the demote cap is reached | `park_and_blank_window` (`window_pool.rs:1092-1152`): hidden, `about:blank`, unregistered | kept (~20–30 MB) |
| Floating pane (cold or promoted) | `try_close_browser`, then `WM_CLOSE` → `DestroyWindow` of the popup; the child browser dies with its parent HWND (`floating_pane.rs`) | freed |

**Why recycling exists:** on CEF 148 a Views browser never fires `OnBeforeClose` on destroy, so it parks with its renderer alive. No close API reaches it again (retro rounds 2–5). Recycling turned that unreachable renderer into a reusable one.

**The demote cap** is `POOL_TARGET_SIZE + 2` = 4 under Normal pressure and 2 under pressure (`window_pool.rs:118-134`). It counts `pool.unpromoted ∪ pool.queue` (`:969-972`).

## 3. Why renderers accumulate

### 3.1 Evicted recycled windows become zombies (the main cause)

- Under commit pressure the heartbeat evicts the whole window pool (`memory_heartbeat.rs`, `while evict_idle_pool_window(&state) {}`).
- `DestroyPoolHwndTask` (`window_pool.rs:303-363`) calls `close_browser(1)` and `DestroyWindow`.
- A recycled window is a Views browser, so it parks. After `EVICTION_CLOSE_GRACE_MS` (2 s), `reap_unclosed_evicted_pool_browser` (`:392-410`) unregisters it: "on_before_close never came".
- Its page is never blanked. It stays "AgentMux (pre-warmed window)", outside the pool sets, the `browsers` map, the renderer map and every count.
- The pool then refills with new windows, which closes recycle into again, and the next pressure episode strands up to 4 more.
- **Seven zombies plus the live windows accounts for all 12 renderers.**

#4767 sheds the pools only at Critical, not Warn, which makes these episodes rarer. Each one still strands renderers.

### 3.2 Each recycle keeps the previous page (very likely; confirm in §4)

- **Demote navigates.** `load_url` of the pool URL, a different URL from the promoted page's (promote only `history.replaceState`s away `pool=1`). So every demote adds a history entry.
- The old page is eligible for Chromium's back/forward cache. `BackForwardCache` isn't in the host's `--disable-features` (`crates/cef/src/app/mod.rs:711-717`), and the cache keeps 6 entries per tab by default.
- **The numbers fit:** 41 demotes over 7 windows is about 6 each, and 1 current + 6 cached pages × 2 documents ≈ 13.
- AgentMux is a single-page app that never navigates back, so the cache is pure cost here.

### 3.3 A recycled renderer keeps its high-water mark

Even with 3.2 fixed, a renderer that once hosted a workspace stays larger than a fresh one, because its allocators keep their pages (the Blink-heap fragmentation the root-cause report measured). That is the 115–124 MB against 52–58 MB.

### 3.4 Side bugs found on the way

- **A floating pane's close skips its label-based cleanup.**
  - The fallback in `CloseWindowTask` (`ui_tasks/window.rs:429-441`) unregisters the browser synchronously, before `on_before_close`.
  - When `on_before_close` then fires, its label lookup (`client/lifecycle.rs:1342-1351`) finds nothing, so it skips `backend_close_window`, the `window_meta` removal, `EvictFloatingPaneWindowState` and the keep-above registration (`:1361-1525`).
  - Small per-close state leaks, including a server-side window row.
- **Adopted windows leak a `POOL_WINDOW_VIEWS` entry on eviction.** `on_pool_window_destroyed` returns early for labels not starting `window-pool-` (`window_pool.rs:720`), so an adopted `window-{uuid}` keeps its cached `cef::Window` entry.

## 4. Measured on CEF 154

### 4.1 The back/forward cache is the main cause (§3.2 confirmed)

Same build (`main`, 2026-10-13), a fresh instance each, the lab's window churn (8 widgets, each opened as a new window and as a floating pane, 3 repetitions, 24 demotes), then every page's document count:

| | `BackForwardCache` on (the host today) | off (`--disable-features=…,BackForwardCache`) |
|---|---|---|
| Each recycled pool window | **16 documents**, ~10,800 DOM nodes, JS heap 42 MB | **1 document**, 143 nodes, heap 11 MB |
| All CEF processes after the churn | 804 MB (from 405) | 527 MB (from 405) |

A longer run of 63 demotes on the same build reproduced the §1 picture: three recycled windows at 13 documents each and renderers of about 150 MB, against 55 MB for a fresh pool window.

### 4.2 A replacing navigation is enough

Turning the cache off for the whole host would take instant Back/Forward away from browser panes, so the fix is narrower. Two idle pool pages, 8 times each: the URL changed with `history.replaceState` (what promotion does), then navigated back to the pool URL. After forcing a GC:

| Navigation | Documents | History entries |
|---|---|---|
| `Page.navigate` (what `load_url` does) | **7** (6–8 before and after GC) | 9 |
| `location.replace` | **1** | 1 |

A replacing navigation leaves no entry to restore, so the cache has nothing to keep. Navigating to a page's own URL is a reload and keeps nothing either, which is why a test that doesn't change the URL first shows no difference.

### 4.3 Result of the fix

`demote` and `park_and_blank_window` navigate with `location.replace`. The same churn on the rebuilt host (a local portable of `main` plus this change):

| | before (24 demotes) | after (24 demotes) | after (64 demotes) |
|---|---|---|---|
| Each recycled pool window | 16 documents, ~10,800 nodes | 1 document, 140 nodes | 1 document, 140 nodes |
| All CEF processes | 405 → 804 MB | 413 → 453 MB | 201 → 459 MB |

The 63-demote run on the old host had three recycled windows at 13 documents each and renderers of about 150 MB. The two "on_before_close never came" evictions (§3.1) still happen, so change 1 is still needed; they now strand a renderer of about 55 MB rather than 150 MB.

Still to measure on CEF 154 (they decide §5 change 1): whether `close_browser` on a blanked Views window fires `OnBeforeClose`, and whether a clean close exits the renderer.

## 5. The changes

**1. Never strand a renderer on eviction.**
- An evicted Views pool window can't be destroyed (unless §4 finds a destroy that works on 154). So instead of `DestroyPoolHwndTask`, it goes to a **bounded parked list**: blanked (`about:blank`), hidden, still registered under `pool.parked`.
- When the pool next spawns a window, it takes a parked one first and loads the pool URL into it, instead of creating a new Views window. A recycle costs a navigation, not a renderer.
- Parked windows count toward the demote cap. When the parked list is at its cap (2), a further eviction falls back to today's park-and-blank.
- **What this guarantees:** the number of live renderers is bounded by the peak number of windows ever open plus the caps, not by the number of pressure episodes.
- If §4 shows that `close_browser` on a blanked Views window does fire `OnBeforeClose` on CEF 154, use that instead, and destroy parked windows beyond the cap.

**2. Don't let a recycle keep the old page (done).**
- Demote and park-and-blank navigate with `location.replace` (`replace_page` in `commands/window_pool.rs`), not `load_url`. Verified in §4.2; the host's `--disable-features` list is unchanged, so browser panes keep the cache.

**3. Recycle within a bound.**
- A recycled renderer above a size threshold (2× a fresh pool window's private working set, measured at demote from the renderer map) isn't demoted. It's parked and blanked, freeing most of its high-water mark.
- With change 1, the next pool spawn reuses it as a fresh page.

**4. Fix the side bugs.**
- **The floater close order:** do the label-based cleanup in `CloseWindowTask`'s fallback before unregistering, or have `on_before_close` look up a closing floater's label by browser id after the unregister. Then `backend_close_window`, `window_meta` and the floating-pane state are released.
- **Adopted windows:** remove the `POOL_WINDOW_VIEWS` entry for any `window-` label, not only `window-pool-`.

## 6. Tests

**Reducer** (`crates/cef/src/reducer/tests.rs`):
- eviction moves a pool window into `pool.parked`;
- spawning takes a parked window before creating one;
- parked windows count toward the cap;
- above the cap, eviction falls back to park-and-blank.

**Close-order unit test:** a floater's close releases its `window_meta` and floating-pane state, and the server-side window row is closed.

**Lab (the measure that matters):**
- the §1 churn run: 60 cycles of new window + floating pane, forced pressure episodes included (`AGENTMUX_CEF_EXTRA_FLAGS` can't force pressure, so run while commit is high, as on the operator's machine);
- record renderer count, the CEF processes' private working set, and documents per recycled renderer;
- **expected:** renderer count bounded by peak windows + caps, no window above 2 documents, and CEF memory back within 20% of its starting point after the windows close.
