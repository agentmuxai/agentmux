# REPORT — Agent pane loading UI: why it isn't rock solid

**Date:** 2026-09-20
**Type:** Analysis (measured live via CDP against a `task dev` instance). No code change.
**Status:** proposed — findings and recommendations only; nothing here has shipped.
**Scope examined:** `frontend/app/block/block.tsx` (the `ready()` gate and its
BrainSpinner), `frontend/app/block/blockframe.tsx` (pane chrome, header mic),
`frontend/app/view/agent/agent-view.tsx` (`.agent-pane-loading-overlay` and its
five-stage reveal), `frontend/app/view/agent/styles/_loading-overlay.scss`.
**Related:** `REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md`,
`SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`,
`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`,
`SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`,
`SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md` (§F below).

---

## 1. User report

> "lets make sure when an agent is loading the loading UI is clean … I see a stray
> microphone icon even during loading, we also sometimes see glitches and composer
> docks appear. it needs to be rock solid."

## 2. Method and its limits — read this before trusting §4

A dev instance was driven over CDP. A new Agent pane was mounted and the DOM sampled
once per animation frame for 1.6s, recording only state *transitions*.

**What this captured:** a fresh pane mount that lands on the agent picker.
**What it did NOT capture:** a *persisted-session* agent load — transcript replay, auth
settling, subagent backfill. That is the longer path, and it is very likely where the
user's symptoms actually live. The agents in the test instance all show
"(missing account)", so a real session load could not be triggered.

Consequently: §4.1-§4.4 are measured or read directly from code. **§5 (the stray mic) is
NOT reproduced** — it is a code-level hypothesis with a stated way to confirm it.

## 3. Measured timeline — new Agent pane, frame resolution

72 frames sampled; 5 distinct states:

```
   1ms  views:3  spin:0  ovl:-        picker:1   frames:8
  58ms  views:4  spin:1  ovl:solid    picker:1   frames:8   <- view + overlay appear together
 126ms  views:4  spin:1  ovl:solid    picker:2   picker mounts UNDERNEATH the overlay
 243ms  views:4  spin:1  ovl:fading   picker:2   fade begins
 562ms  views:4  spin:0  ovl:-        picker:2   overlay unmounted
```

The covered-then-revealed sequence works as designed here: the picker mounts at 126ms
while the cover is still opaque, and is revealed by the fade. An earlier 100ms-resolution
pass of the same mount also recorded **`spin:2`** — two BrainSpinners on screen
simultaneously (see §4.1).

## 4. Findings

### 4.1 Two independent loading systems, each with its own spinner and lifetime

There are two entirely separate "this pane is loading" mechanisms:

| | Block level | Agent-view level |
|---|---|---|
| gate | `ready()` = `!loading() && blockData() != null && viewModel() != null` (`block.tsx:378`) | `showLoadingOverlay()` (`agent-view.tsx:834`) |
| UI | `<BrainSpinner fading={spinnerFading()} />` (`block.tsx:498`) | `.agent-pane-loading-overlay` + `<BrainSpinner>` (`agent-view.tsx:2166-2171`) |
| covers | the block's content box | `.agent-view`'s positioned box only |
| ends on | `ready()` + `subagentBackfillSettled()` | a five-stage reveal (§4.2) |

Neither knows about the other. They can be on screen at the same time — measured:
`spin:2`. Two brains fading on independent clocks over the same pane is the most likely
source of "glitches" as a *class*, independent of any single bug: there is no single
authority for "is this pane still loading", so there is no single moment at which the
pane is allowed to appear.

### 4.2 The reveal is a five-stage pipeline with independent timers

`historyPainted()` → `authPhaseSettled()` → `scheduleOnSettle` (Long-Task quiet
detector) → two extra `requestAnimationFrame`s → set `historyLoaded()` (starts the CSS
fade) → `loadingOverlayFadeTimeout` (220ms) → `setShowLoadingOverlay(false)` unmounts.

Each stage exists for a real reason documented in-place (the auth-panel pop-in fix, the
codex P1 about the overlay's own background not fading, the "Long-Task quiet is reached
before the browser has painted" note). But the composition is five independent async
sources feeding one boolean. Any of them firing early reveals the pane mid-assembly.
This is the architecture the user is sensing when they say it needs a rethink: the
pane's readiness is *inferred* from a chain of proxies rather than *stated*.

### 4.3 The overlay covers its positioned ancestor — not "the pane"

`.agent-pane-loading-overlay` is `position: absolute; inset: 0`
(`_loading-overlay.scss:10-12`), so its coverage is whatever the nearest positioned
ancestor happens to be. That is an implicit contract with the DOM shape: **any future
restructuring silently changes what the loading UI hides.** Nothing asserts the
coverage.

This is not hypothetical — see §F.

### 4.4 Pane chrome is deliberately outside the overlay

The overlay's own comment records that it must NOT cover `.pane-tab-strip`, so the
"+"/tab controls stay usable during load
(`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md §1.4`), and the progress bar is
portaled outside `.agent-view` entirely. That is a reasonable product decision, but its
consequence is architectural: **every header-level element is visible during loading and
must suppress itself independently.** There is no "pane is loading" signal available to
chrome components at all. Each one is on its own.

## 5. The stray microphone — hypothesis, NOT reproduced

`blockframe.tsx:270`:

```tsx
<Show when={props.viewModel?.voiceHandle && props.blockView !== "agent"}>
    <MicButton ... />
```

Agent panes are supposed to render their mic beside the composer instead
(`SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`); terminals keep
the header mic. The suppression is **negative** — "show unless this is an agent".

`blockView` comes from `blockData()?.meta?.view` (`blockframe.tsx:581`), and `blockData()`
is `null` while its MuxObject is still loading. So in that window the guard evaluates
`undefined !== "agent"` → **true**, and the header mic renders if the view model in hand
exposes a `voiceHandle`. The same un-resolved value also makes `getViewElem` return
`<CenteredDiv>No View</CenteredDiv>` (`block.tsx:88-90`) — another placeholder visible in
the same window.

Both `BlockFrame` call sites are gated (`BlockPreview` early-returns on null data;
`BlockFull` sits inside `<Show when={ready()}>`), which is why this did not reproduce in
§3. Candidate windows not yet tested: a pane whose block data resolves *after* the frame
mounts (reconnect, cross-window open, stale cache entry), or a view-type change in place.

**Verified NOT the cause in steady state:** the two header mics present during testing
both belong to terminal panes (`paneTitle: "C:\Program Files\PowerShell\7"`), which is
by design.

**To confirm:** log `{blockId, blockView, hasVoiceHandle}` at `blockframe.tsx:270` on
every evaluation, open an agent pane, and check whether any evaluation has
`blockView == null` while `hasVoiceHandle` is true. That is a one-line dev-server change
— no rebuild needed.

## 6. "Composer docks appear"

Not reproduced in §3 either — the composer strip and the drawer were already mounted for
pre-existing panes throughout, and the new pane's picker mounted *under* the opaque
overlay. The plausible mechanism is §4.3 + §4.4: anything that mounts outside the
overlay's positioned ancestor, or after the overlay has already begun fading, appears as
a pop-in. Given §4.2's five independent timers, "after the fade started" is easy to hit.

## 7. Recommendations — the rethink

1. **One readiness authority per pane.** Collapse the block-level `ready()` gate and the
   agent-view overlay into a single state machine that owns "assembling → revealing →
   live", and have both spinners read from it. Today two systems each decide
   independently, and §3 measured them overlapping.
2. **Make readiness stated, not inferred.** Replace the five-proxy chain (§4.2) with
   explicit completion signals from the things being waited on. A pane should reveal
   because every declared dependency reported done, not because a Long-Task detector went
   quiet and two rAFs elapsed.
3. **Make the cover's scope explicit.** The overlay should declare what it covers rather
   than inheriting it from whichever ancestor happens to be positioned (§4.3, §F). A test
   asserting "during loading, the drawer/composer/etc. are covered" would have caught §F
   immediately.
4. **Make chrome suppression positive.** `blockView !== "agent"` shows the mic whenever
   the view is *unknown*. Inverting it — render the header mic only for view types known
   to want it (`blockView === "term"`) — makes the unresolved state render nothing, which
   is the safe default for every such guard, not just this one.
5. **Give chrome a loading signal.** §4.4 notes chrome has no way to know the pane is
   loading. Publishing that would let the header suppress transient affordances without
   each component inventing its own guard.

Items 4 and 5 are small and independently shippable; 1-3 are the actual architecture
work.

## F. Regression introduced today — must fix before the drawer change lands

`SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md` moved the Shell drawer out
of the per-pane `zoom` by introducing `.agent-view-zoomed` and making the drawer a
sibling of it. That wrapper was given `position: relative`, and the loading overlay lives
*inside* it.

Per §4.3, the overlay's coverage is its nearest positioned ancestor — which is now the
wrapper, not `.agent-view`. **The drawer is therefore no longer covered by the loading
overlay.** An open drawer will be visible, uncovered, for the whole load.

This is a direct instance of §4.3's implicit contract breaking silently under
restructuring, found by reviewing the loading path rather than by any test. Options:
move the overlay to be a sibling of the wrapper (covering `.agent-view` again), or drop
`position: relative` from the wrapper if nothing else needs it — either way, add the
coverage assertion from recommendation 3 so this cannot regress again.
