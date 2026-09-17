// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { PaneMediaPermissionPrompt } from "@/app/window/pane-media-permission-prompt";
import { PaneMediaCaptureIndicator } from "@/app/window/pane-media-capture-indicator";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms } from "@/store/global";
import { gateTargetTabId, scheduleRevealLift, tabSwitching } from "@/store/tab-reveal";
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import type { JSX } from "solid-js";

function WorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;
    const prefersReducedMotion = atoms.prefersReducedMotionAtom;

    // Tab container elements by tab id, for the forced-layout effect below.
    const tabEls = new Map<string, HTMLDivElement>();

    // Displayed tab id — mirrors `tabId()`, but its OWN update is wrapped in
    // `document.startViewTransition()` so the swap uses Chromium's real
    // cached-snapshot cross-fade (ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_
    // 2026_09_15.md) instead of a hard cut. `tabId()` itself is untouched —
    // it stays the backend-authoritative signal `gateHides` and everything
    // else already depends on. Only the content-visibility/pointer-events
    // bindings below read this derived signal; it's a presentation-only
    // layer, not a new source of truth. Feature-detected: View Transitions
    // shipped in Chromium 111, well within this app's CEF baseline
    // (browserslist "Chrome >= 128"), but detected rather than assumed — a
    // missing API degrades to the plain instant swap, not a throw.
    //
    // Reduced-motion (reagent P1 on PR #3239): the removed opacity fade was
    // explicitly gated on this same atom (Codex P2 on PR #1108), and that
    // gating was accidentally dropped along with the fade itself. The
    // app-wide `.prefers-reduced-motion` CSS override (app.scss) only zeroes
    // `transition-*` properties, not the `animation-*` ones a view
    // transition's `::view-transition-old/new` pseudo-elements actually
    // animate with — so skip calling startViewTransition at all when the
    // user prefers reduced motion, same as the code this replaced did for
    // its own animation.
    //
    // Reveal-gate resync (reagent P1 on PR #3239): `document.
    // startViewTransition()`'s update callback is NOT guaranteed to run
    // synchronously with the call — the browser queues it (spec: a task on
    // the DOM manipulation task source), so `setDisplayTabId` can land an
    // unbounded-but-usually-short interval after `tabId()` itself flips.
    // Meanwhile `tab-actions.ts`'s `scheduleRevealLift()` starts its OWN
    // settle-detector clock the moment `tabId()` flips, independent of
    // `displayTabId()`. If that clock decides "settled" (80ms of no long
    // tasks) before `displayTabId()` has caught up, the gate can lift while
    // the destination tab is still content-visibility:hidden — so its
    // eventual real first paint, once content-visibility DOES flip, happens
    // completely unmasked, reintroducing the exact FOUC the gate exists to
    // prevent. Fix: re-arm the settle detector (idempotent — see
    // scheduleRevealLift's own doc comment, "a second call... resets the
    // detector") right when displayTabId actually catches up, so the clock
    // is always anchored to the real content-visibility flip, not to
    // whenever the RPC happened to resolve. Only if a gate is already
    // active (`tabSwitching()`) — never arms a NEW gate for a switch
    // nothing else decided to gate (e.g. backend-driven switches that
    // bypass setActiveTab entirely, per that file's own comment).
    const [displayTabId, setDisplayTabId] = createSignal(tabId());
    createEffect(() => {
        const next = tabId();
        if (next === displayTabId()) return;
        const apply = () => {
            setDisplayTabId(next);
            if (tabSwitching()) scheduleRevealLift();
        };
        if (!prefersReducedMotion() && typeof document.startViewTransition === "function") {
            document.startViewTransition(apply);
        } else {
            apply();
        }
    });

    // Force the just-activated tab's layout to complete SYNCHRONOUSLY,
    // in this same effect, rather than letting content-visibility defer it.
    //
    // Why this is needed (user report, 2026-09-15): "the tab shows before
    // the panes are fully set" — subtle glitches visible after switching.
    // `content-visibility: hidden -> visible` does not force layout the
    // instant the style flips; the browser is free to do that work on a
    // LATER frame, and it can arrive split across several sub-50ms chunks
    // instead of one blocking task. The reveal gate's settle detector
    // (tab-reveal.ts) only knows a tab has finished settling by watching
    // for a quiet window with no PerformanceObserver `longtask` entries —
    // work that never crosses the 50ms threshold is invisible to it, so
    // the gate can lift (revealing the tab) while genuine layout/paint
    // catch-up is still happening on subsequent frames.
    // SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md's own investigation
    // log already named this exact shape of gap under the OLD
    // `display:none` mechanism too ("a separate 500-600ms long-task fires
    // AFTER the reveal gate lifts... the reveal gate's tab-switch number
    // is misleadingly small") — content-visibility didn't invent this race,
    // it just removed the 120ms opacity fade's incidental masking of it
    // (that fade was blending exactly these "un-cascaded" post-reveal
    // frames into invisibility, per its own now-removed comment).
    //
    // Reading a layout-dependent property (`getBoundingClientRect`) on the
    // tab's own container right after its `content-visibility` flips to
    // "visible" forces Chromium to synchronously complete that tab's
    // pending layout in THIS task, before the settle detector's first
    // `requestAnimationFrame` poll — turning a possibly-deferred, possibly-
    // sub-threshold cost back into one measurable, gate-respecting task,
    // the same character the settle detector was originally built around.
    createEffect(() => {
        const id = displayTabId();
        const el = tabEls.get(id);
        if (el) void el.getBoundingClientRect();
    });

    // Reveal gate, destination-aware (SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH §9):
    // the holder always announces WHICH tab is being revealed
    // (gateTargetTabId) — only that tab hides while gated, so the SOURCE
    // tab keeps painting right up to the activetabid flip instead of
    // blanking the whole content region for the RPC round trip. No
    // untargeted form anymore (SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_
    // 2026_09_16.md) — createTab used to hold this gate untargeted
    // because its destination tab didn't exist yet; it now doesn't hold
    // this gate at all until the destination's content already exists.
    const gateHides = (tid: string) => {
        if (tid !== tabId() || !tabSwitching()) return false;
        return gateTargetTabId() === tid;
    };

    // All tab IDs (pinned + regular). Keep every tab mounted so terminals
    // preserve their xterm.js instance and scrollback across tab switches.
    // Inactive tabs are hidden via content-visibility:hidden — no
    // unmount/remount, and (unlike the display:none this replaced) no
    // discarded layout either. See the content-visibility comment below.
    const allTabIds = createMemo<string[]>(() => {
        const w = ws();
        if (!w) return [];
        return [...(w.pinnedtabids ?? []), ...(w.tabids ?? [])];
    });

    return (
        <div class="flex flex-col w-full flex-grow overflow-hidden">
            <WindowHeader workspace={ws()} />
            <div
                class="flex flex-row flex-grow overflow-hidden"
                style={{
                    "min-height": 0,
                    position: "relative",
                    // Scopes the view transition above to just this region —
                    // see app.scss's ::view-transition-old/new(workspace-tab-content)
                    // rule. Without a name, document.startViewTransition()
                    // snapshots the WHOLE document by default, briefly
                    // freezing the window header and status bar into the
                    // crossfade too.
                    "view-transition-name": "workspace-tab-content",
                }}
            >
                <ErrorBoundary>
                    <Show when={allTabIds().length > 0} fallback={<CenteredDiv>No Active Tab</CenteredDiv>}>
                        <For each={allTabIds()}>
                            {(tid) => {
                                onCleanup(() => tabEls.delete(tid));
                                return (
                                <div
                                    ref={(el) => tabEls.set(tid, el)}
                                    class="flex flex-row h-full w-full"
                                    style={{
                                        // Absolutely positioned, stacked on top of each other,
                                        // filling the relative-positioned parent above — NOT a
                                        // flex-row sibling of the other tabs. Every tab div is
                                        // now permanently `display: flex` (content-visibility
                                        // below is what hides inactive ones), and the parent
                                        // container is `flex flex-row` — without taking each tab
                                        // out of normal flow, N simultaneously-`display:flex`
                                        // siblings each wanting `width: 100%` get their widths
                                        // negotiated down by flexbox to fit one row, i.e. every
                                        // tab's content visibly compresses horizontally as more
                                        // tabs open. `position: absolute` + `inset: 0` removes
                                        // each tab div from the parent's flex layout entirely, so
                                        // only the content-visibility:visible one is ever
                                        // meaningfully occupying the area — same fix shape as
                                        // "keep mounted, show one" UI generally uses.
                                        position: "absolute",
                                        inset: "0",
                                        // ALWAYS "flex" now — content-visibility below is what
                                        // hides an inactive tab, not display. See the
                                        // content-visibility comment for why: `display:none`
                                        // removes the subtree from layout entirely, so
                                        // reactivating a tab meant the browser laying out and
                                        // painting it from zero, same as first insertion.
                                        display: "flex",
                                        // ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md:
                                        // an inactive tab's whole content div is
                                        // content-visibility:hidden rather than display:none —
                                        // SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md measured
                                        // the display:none/flex flip costing 500-600ms of
                                        // browser-side layout+paint per switch, confirmed (by
                                        // elimination of every JS-side hypothesis) to be the
                                        // engine laying out the revealed subtree as if newly
                                        // inserted — because display:none actually removes it
                                        // from the render tree, discarding any prior layout.
                                        // content-visibility:hidden skips rendering while
                                        // inactive (same "no ongoing cost while backgrounded" as
                                        // display:none) but keeps a cached rendering state, so
                                        // reactivating reuses it instead of starting from zero —
                                        // Chromium's own documented purpose for this property is
                                        // literally "tab-like UI." A DIFFERENT value,
                                        // content-visibility:auto, was already tried at the
                                        // per-ROW level inside the virtualized document list and
                                        // made things worse (auto's per-frame near-viewport
                                        // polling fighting the virtualizer's own visible-range
                                        // math) — that result doesn't predict this one: `hidden`
                                        // is a binary switch with no per-frame polling, applied
                                        // here at the tab-container boundary where nothing else
                                        // is already deciding visibility. Also incidentally
                                        // closes REPORT_TAB_FLASH_SYSTEMIC_ANALYSIS_2026_08_31.md
                                        // §3.4's 0x0-measurement/ResizeObserver cascade: unlike
                                        // display:none, a content-visibility:hidden element with
                                        // an explicit (non-content-derived) size — h-full/w-full
                                        // here, not "auto" — keeps that real size the whole time
                                        // it's hidden, so there's no 0x0-to-real-size jump left
                                        // for a ResizeObserver to fire on when it reactivates.
                                        // Never needs a `display:none` fallback: AgentMux always
                                        // runs on CEF/Chromium, which has supported
                                        // content-visibility since the property shipped.
                                        "content-visibility": tid === displayTabId() ? "visible" : "hidden",
                                        // content-visibility:hidden skips rendering, but does
                                        // NOT imply pointer-events:none — an inactive tab's div
                                        // is still a real, absolutely-positioned box stacked on
                                        // top of the active one, and un-painted boxes still
                                        // hit-test. Without this, whichever tab is LAST in DOM
                                        // order (i.e. the most-recently-opened one) sat on top
                                        // in stacking order and silently swallowed every click
                                        // meant for whatever tab was actually showing beneath
                                        // it — the exact "tabs 1-3 stopped being clickable, tab
                                        // 4 still worked" report from adding position:absolute
                                        // above. Only the active tab may receive pointer events;
                                        // every inactive one lets clicks pass through to it.
                                        "pointer-events": tid === displayTabId() ? "auto" : "none",
                                        // Reveal gate (issue #774): hide the active tab while
                                        // it's still settling so the piecemeal mount cascade
                                        // doesn't paint stage-by-stage. `visibility: hidden`
                                        // preserves layout and suppresses paint without
                                        // unmounting children. Lifted by `tab-reveal.ts`'s
                                        // frame-budget detector. Only applies to the active
                                        // tab — inactive tabs are `content-visibility: hidden`
                                        // already.
                                        //
                                        // No opacity fade on lift anymore (removed 2026-09-15,
                                        // user report against the content-visibility +
                                        // position:absolute rework above): the 120ms opacity
                                        // transition was animating a tab whose layout box is now
                                        // also interacting with content-visibility's own cached-
                                        // state reuse, and the combination read as part of the
                                        // "blocks compress/settle visibly" complaint the absolute-
                                        // positioning fix was already chasing. `visibility`
                                        // flips instantly with no animation, so this is a snap,
                                        // not a fade, on every platform including reduced-motion.
                                        visibility: gateHides(tid) ? "hidden" : null,
                                    }}
                                >
                                    <ErrorBoundary>
                                        <TabContent tabId={tid} />
                                    </ErrorBoundary>
                                </div>
                                );
                            }}
                        </For>
                    </Show>
                    <ModalsRenderer />
                    {/* Camera/mic prompts for browser panes. Mounted in the
                        main window's DOM so the requesting page cannot draw or
                        click it — see the component's own doc comment and
                        SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md §3.5. */}
                    <PaneMediaPermissionPrompt />
                    <PaneMediaCaptureIndicator />
                </ErrorBoundary>
            </div>
            <StatusBar />
        </div>
    );
}

export { WorkspaceElem as Workspace };
