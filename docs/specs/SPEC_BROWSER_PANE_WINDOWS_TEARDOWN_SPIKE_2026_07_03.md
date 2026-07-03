# SPEC: Windows browser-pane renderer teardown — Phase-0 spike scope

Status: draft (spike scoping — no code changes)
Date: 2026-07-03
Owner: Lzop
Tracking: #1936 (renderer/pagefile commit-charge leak)
Prior art: `docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md` (2026-04-17),
`docs/specs/SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md`

## 1. The bug this spike targets

On Windows, closing an embedded browser pane never tears down its CEF
renderer process. `agentmux-cef/src/browser_panes.rs::close()`/`close_with`
(current code, ~L386-496) destroys the pane's Win32 child HWND and drops our
`Arc<Browser>`, but deliberately never calls `host.close_browser()` — so
nothing tells CEF's Alloy runtime to actually release the renderer. The
Browser's internal refcount may eventually reach zero and fire
`on_before_close` (the code comment says "may... eventually"), but there is
no guarantee, and no forcing function. This is the confirmed mechanism
behind #1936's "5 renderer processes survive closing panes to 1, commit
unchanged" observation — full investigation posted on that issue.

Linux/macOS do not have this bug: `browser_pane/creation_views.rs::detach_browser_pane_view`
(~L557-609) calls `host.close_browser(1)` and gets a clean, real teardown via
CEF's normal `on_before_close` path. This spike is Windows-only.

## 2. Why this isn't a quick fix — prior art (read this before touching the code)

The current `DestroyWindow`-only approach is not an oversight. It is the
outcome of **three different `close_browser()`-based fixes, all tried and
abandoned within about two hours on 2026-04-18** (`git log` on
`browser_panes.rs` / `client.rs`, commits `b6b9588e` → `f9994706` →
`81391dcc` → `4447c063`):

| # | Time (2026-04-18) | Approach | Result |
|---|---|---|---|
| 1 | 23:04 (04-17) | `host.close_browser(1)` (force) | Cascaded into main's `on_before_close`; **quit the whole app**. |
| 1b | " | `host.close_browser(0)` (graceful) | Same cascade, less aggressively — main still torn down. |
| 2 | 01:51 | Explicit `Live/Closing` state gate on the pane entry (fixes a **different**, real bug: `SetFocus`/`resize` hitting a mid-destruction HWND) | Fixed its own crash; did not fix the cascade — still called `close_browser(0)` underneath. |
| 3 | 02:08 | `PANE_CLOSE_IN_PROGRESS` atomic counter + `do_close` guard: when a pane close is in flight, main's `do_close` returns `true` (cancel) | **Root mechanism discovered here**: CEF/Alloy treats `close_browser(pane)` as *one atomic teardown* when the pane's outer HWND is a `WS_CHILD` of main's top-level. Cancelling either side's `do_close` cancels **both**. Result: app stayed up, but now the *pane's* renderer also survived — back to the original bug, from a different code path ("app doesn't close, but the browser content remains"). |
| 4 | 02:55 | `DestroyWindow` on the pane's own HWND instead of `close_browser()` at all | **What ships today.** Avoids the cascade entirely by never invoking Alloy's coupled teardown path — at the cost of never getting a real renderer teardown either. |

**The load-bearing finding from attempt 3** (commit `4447c063`'s message,
verified still accurate against today's code — no `is_pane` guard or
`PANE_CLOSE_IN_PROGRESS` remain anywhere in `client/`):

> CEF treats `close_browser(pane)` as one atomic teardown that fires
> `do_close` on both the pane browser AND main. Cancelling either cancels
> both.

This means the coupling is **not** a race you can win with better
sequencing or a smarter guard at the callback level — three attempts at
exactly that already failed. The pane and main browser are structurally
tied together *because* the pane's CEF `WindowInfo` was created via
`set_as_child(main_hwnd, rect)` (`browser_pane/creation.rs:145`), making the
pane's outer HWND a literal Win32 `WS_CHILD` of main's top-level. Any
approach that keeps that parent/child relationship intact while calling
`close_browser()` on the pane is very likely to rediscover the same
coupling. **Do not re-attempt approaches 1-3 verbatim** — they are a matter
of record, not a hunch.

## 3. Candidate designs for the spike

### Candidate A — detach-before-close (reparent, then destroy) — untried for embedded panes, but backed by a WORKING precedent already in this codebase

**This is not a novel hypothesis — it is a close mirror of a pattern that
already ships and works, for a structurally adjacent case: floating
(torn-off) panes.** `agentmux-cef/src/floating_pane.rs` (~L4-14, ~L726-743)
embeds a browser pane inside a `WS_POPUP` Win32 window that has **no Win32
owner** (explicitly not a child of main — issue #1560 removed the owner
relationship specifically to fix z-order bugs). Its documented close path:

> 1. User clicks X → `DefWindowProcW(WM_CLOSE)` → `DestroyWindow` (on the
>    floater's OWN un-owned top-level HWND, not a WS_CHILD of anything)
> 2. Outer HWND's `WM_DESTROY` cascades into the CEF child HWND (which
>    *is* `WS_CHILD` of the floater's own outer HWND)
> 3. CEF's wndproc on the child runs its destroy handler → `OnBeforeClose`
>    fires on `AgentMuxHandler` → reducer `UnregisterBrowser` cleans
>    `state.browsers` + `window_meta` — a **clean, complete teardown**.

This works precisely *because* the HWND being destroyed is a genuine,
un-owned top-level window — not a `WS_CHILD` of main. Compare to the
buggy embedded-pane path: `browser_pane/creation.rs:145`'s
`set_as_child(main_hwnd, rect)` makes the *embedded* pane's outer HWND a
literal `WS_CHILD` of **main's** top-level, and destroying (or
close_browser-ing) a non-top-level `WS_CHILD` that CEF didn't create as an
independent top-level is exactly the case CEF's own docs warn is
unreliable (`SPEC_BROWSER_PANE_LIFECYCLE.md` §8: "`DoClose` is NOT called
when the host window is destroyed via parent hierarchy tear-down").

**Hypothesis (sharpened from the original detach-then-close_browser idea):**
before closing an *embedded* pane, first promote its outer HWND out of the
`WS_CHILD`-of-main relationship — e.g. `SetParent(pane_hwnd, NULL)` (making
it a genuine top-level, un-owned window, matching the floater's own
structure) — then destroy it the same way the floater does (`DestroyWindow`
on that now-top-level HWND, letting `WM_DESTROY` cascade into CEF's own
child teardown and fire `OnBeforeClose` normally). This does **not**
necessarily need an explicit `close_browser()` call at all — the floater
precedent gets a clean teardown via `DestroyWindow`-on-your-own-top-level,
not via `close_browser()`. That may sidestep the entire Alloy
`close_browser(pane)`-conflates-with-main coupling from §2, since that
coupling was specifically about the `close_browser()` API call, not about
`WM_DESTROY` cascading through a window's own child hierarchy (which is
the normal, correct pattern the floater already proves works on this
exact CEF/Alloy build).

This is genuinely untried *for embedded panes* — `git log
--grep="reparent|SetParent|detach.*hwnd"` across the whole repo history
turns up nothing in `browser_panes.rs`/`browser_pane/`. But structurally,
it's the closest thing to a proven pattern this spike has, since the
floater is effectively "Candidate B, achieved on Windows via raw Win32
rather than CEF Views" — already shipping, for a different code path.

Open questions a spike must answer:
- Does `SetParent(pane_hwnd, NULL)` on a live CEF-owned `WS_CHILD` HWND
  work cleanly mid-session (the floater is *created* with no owner from
  the start — this spike needs to *transition* an existing embedded pane
  out of its WS_CHILD relationship after the fact, which is a different,
  untested operation)? Does the render surface keep compositing correctly
  through the reparent (matters only briefly, since the very next step
  destroys it), or does reparenting itself glitch/corrupt neighboring
  panes' compositor chains?
- Does the resulting un-parented top-level HWND need `WS_EX_TOOLWINDOW` /
  its own window class (matching the floater's setup at
  `floating_pane.rs` `CLASS_REGISTERED`) to destroy cleanly, or is a bare
  reparent-then-`DestroyWindow` on the pane's *existing* class sufficient?
- Whether an explicit `close_browser()` call is still needed/beneficial
  after the reparent+destroy (e.g. to get `beforeunload` to run, which
  the current `DestroyWindow`-only approach explicitly forgoes), or
  whether that reintroduces §2's coupling and should be skipped entirely,
  matching the floater's own approach of never calling it.

### Candidate B — match Linux/macOS: embed panes as CefBrowserViews instead of separate native HWNDs

Instead of `set_as_child` (a real Win32 child window wrapping its own CEF
Browser), give Windows panes the same architecture Linux/macOS already use
successfully — a `CefBrowserView` added as an overlay/child view inside
main's *existing* CEF Views window, sharing its native HWND rather than
creating a new one. This is the architecture that's proven not to have the
cascade bug at all, on two platforms, in production.

This is a bigger lift than Candidate A: Windows currently uses the Alloy
runtime for the main window (not CEF Views) for HWND-precision reasons
documented elsewhere in this codebase (search `client/display.rs`,
`app.rs` for why Windows stays on Alloy). Moving *just panes* to a
Views-embedded model while main stays Alloy may or may not be possible
inside one CEF build — needs its own investigation before this is a real
candidate, not just a design spike.

### Candidate C — harden the current DestroyWindow path (no architecture change)

Lowest risk, does not fix the leak, listed for completeness / as a fallback
if A and B both fail validation. Possible improvements within the current
approach:
- After `DestroyWindow`, poll/verify the renderer process actually exits
  within some bound; if not, consider it a leaked renderer and count it
  (visibility, not a fix).
- Investigate whether there's a CEF API to explicitly release a
  `RequestContext`'s renderer process handle independent of `close_browser`
  (i.e., something that doesn't route through the coupled Alloy teardown
  at all) — would need a CEF API surface audit, not assumed to exist.

## 4. Validation protocol (do not skip this — 3 prior attempts each looked correct on paper)

Every prior attempt in §2 compiled clean, passed review, and *looked*
correct — the bug only surfaced under live manual exercise with a host log
trace. A spike here must be validated the same way, not just code-reviewed:

1. Build via `task package` (or `task dev`) — do not rely on `cargo check`
   alone; the failure modes here are runtime CEF/Win32 behavior, invisible
   to the type checker.
2. Open a browser pane, navigate it somewhere real (not `about:blank` —
   attempt 2's crash only reproduced post-navigation), then close it.
3. **Main window must survive** — the literal regression from attempts 1
   and 1b. Watch for the host log's `on_before_close`/`quit_message_loop`
   sequence (same signature as the `4447c063` trace in §2) to confirm main
   was NOT torn down.
4. **The renderer process must actually exit** — check via `tasklist` (or
   Process Explorer) for the `agentmux-*.exe` render process count
   before/after close, not just "the pane's pixels disappeared." Pixels
   disappearing was already true of the current `DestroyWindow` approach
   and is not evidence of a fixed renderer leak.
5. **`state.browsers` must not retain a stale entry** for the closed
   pane's label — check via existing debug logging/`state.browsers.lock()`
   inspection (or add a temporary debug RPC) — a leaked map entry with a
   dead HWND is a different, quieter version of the same bug.
6. Repeat with **two panes open, close one** — attempts 1/1b/3's cascade
   specifically involved a *second* browser (main) sharing HWND lineage;
   single-pane-only testing would not have caught it.
7. Repeat with a pane that's *mid-navigation* when closed (the H.7
   mid-close invariant elsewhere in this codebase exists precisely because
   concurrent pane-lifecycle + window-creation races are a real failure
   class here — see `commands/window_pool.rs` H.7 comments).

Land behind something reversible for the spike itself (a dev-only code
path, a debug flag, or simply keep it on a branch and not merge until all
of §4 passes) — given the track record, treat "looks right" as
insufficient signal on its own.

## 5. Success criteria

- Main window survives pane close in every scenario in §4.
- The pane's renderer process count actually drops (not just HWND/pixels).
- `state.browsers` has no stale entry for the closed pane after close.
- No regression in the existing Rust (`browser_panes::tests`, L1) or
  frontend (`browser-model.test.ts`, L2) suites from
  `SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md`.
- Focus behaves at least as well as today (§4.3 of the original lifecycle
  spec documents the current focus-reclaim-after-destroy path
  (`reclaim_focus_after_pane_destroy`) — don't regress that while changing
  the close mechanism it's paired with).

## 6. What's explicitly out of scope for this spike

- Active eviction of already-warm, not-yet-promoted window-pool windows
  under memory pressure (#1936's other deferred item) — different code
  path (`window_pool.rs`), different risk profile, not blocked on this.
- The full `PaneLifecycle` module restructure from
  `SPEC_BROWSER_PANE_LIFECYCLE.md` §6-7 — that was *already* explicitly
  deferred in `f9994706` as a follow-up "once this lands and bakes," and
  never happened. Worth reconsidering once a working close path exists,
  not a prerequisite for this spike.
- Candidate B (Views-embedded Windows panes) beyond a feasibility check —
  full implementation is its own spec if A doesn't pan out.

## 7. Files this spike will touch

- `agentmux-cef/src/browser_panes.rs` (`close`, `close_with`, ~L386-496)
- `agentmux-cef/src/browser_pane/creation.rs` (`set_as_child`, ~L145 — where
  the parent relationship is originally established; Candidate A's
  reparent call is the mirror-image operation)
- `agentmux-cef/src/floating_pane.rs` (~L4-14, ~L726-743) — **read as
  reference, not modified**: the working precedent Candidate A mirrors.
  Worth re-reading in full before writing any code, including the wndproc
  (`floating_pane_wndproc`) that handles the floater's `WM_CLOSE`/
  `WM_DESTROY` sequence.
- `agentmux-cef/src/client/lifecycle.rs` (`do_close`, `on_before_close`) —
  read-only for this spike unless Candidate A needs a callback change
- Possibly `agentmux-cef/src/browser_panes.rs` tests (`close_with_*`,
  ~L1117-1128) if `close_with`'s side-effect contract changes
