// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneTabStrip — the shared, pane-type-agnostic tab strip. Extracted from
 * the editor's tab strip (`frontend/app/view/editor/editor-tab-strip.tsx`,
 * `.editor-tab-strip`/`.editor-tab`) so agent-pane forks and terminal-pane
 * shell tabs can reuse the exact same chrome and interaction model instead
 * of each pane type growing its own copy.
 *
 * Deliberately accessor-based rather than requiring tabs to conform to a
 * fixed shape (`{id, label, ...}`) — callers pass closures that read
 * whatever fields their own tab type actually has (e.g. the editor's
 * `EditorTab.filePath`/`.dirty`/`.displayName`, a future fork entry's
 * `.title`/`.blockId`). This keeps `props.tabs` as the caller's own array
 * reference (no per-render remapping into throwaway objects), so Solid's
 * `<For>` keeps its row-reuse identity optimization intact.
 *
 * Presentational only: renders a row of tabs + an optional trailing `+`,
 * reports intent via callbacks. Owns no tab state.
 *
 * Spec: docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §3.1.
 */

import { createEffect, createSignal, For, on, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";
import { draggable, dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { flashElement, onActivityFlash } from "@/app/notification/activity-flash";
import { atoms } from "@/store/global";
import { Tooltip } from "./tooltip";
import "./PaneTabStrip.scss";

// Matches the other reveal-gate/cross-fade durations added alongside this
// one in SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.4.
const WIDTH_TRANSITION_MS = 160;

/** Drag payload tag for a Pane Tab pill, mirroring the existing
 *  `tileItemType`/`tabItemType` module-level constants
 *  (tilelayout-shared.tsx / tabbar-dnd.ts) — a distinct tag so a dragged
 *  pill is never mistaken for a whole-Pane or Window-Tab drag by any
 *  existing drop target.
 *  SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.1. */
export const paneTabItemType = "PANE_TAB_ITEM";

/** How long a just-landed pill keeps `.pane-tab--landing` — the Window Tab
 *  bar's own clear-timeout for its bounce (tab-reorder.ts), so the two match. */
export const LANDING_BOUNCE_MS = 400;

/** The tab (by id) that was just dropped into place and should play the
 *  landing bounce. Module-level rather than per-strip: a cross-pane drop
 *  lands the pill in a DIFFERENT strip instance than the one that handled
 *  the drop (and mounts it fresh there), so the flag has to be readable by
 *  whichever strip ends up rendering that id. Only header strips set it,
 *  and their ids are blockIds, which never collide with the editor's or
 *  History strip's ids.
 *  SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §3.4. */
const [landingTabId, setLandingTabId] = createSignal<string | null>(null);
let landingTimer: ReturnType<typeof setTimeout> | undefined;
function markLanded(id: string): void {
    clearTimeout(landingTimer);
    setLandingTabId(id);
    landingTimer = setTimeout(() => setLandingTabId(null), LANDING_BOUNCE_MS);
}

/** A tab's own pane color, split into the two treatments PaneChrome's
 *  hue/identity-color system already gives a block (blockframe.tsx):
 *  `underline` is the vivid border-strength color (shown only on the
 *  active tab, replacing the generic `--accent-color`); `background` is
 *  the same darkened/muted tone the block's OWN header would show, on
 *  EVERY tab that has one — active included
 *  (SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md §2.2; it used to be
 *  inactive-only, which left the selected tab as the only colorless pill
 *  in the strip once the header behind it stopped carrying the color).
 *  Either can be absent on its own (a block with no color assigned yields
 *  both undefined) — callers fall back to the strip's existing plain
 *  chrome. `neutralBackground` is the opaque resting background for a tab
 *  with no `background` of its own — needed wherever the strip sits on a
 *  tinted surface, where the default transparent pill would show that
 *  tint through. */
export interface PaneTabColors {
    underline?: string;
    background?: string;
    neutralBackground?: string;
}

/**
 * Which side of a hovered pill's rect a dragged pill should land on, given
 * the pointer's clientX. Pure and exported so it's unit-testable in
 * isolation — the actual drag gesture (pragmatic-dnd, real pointer events)
 * has no unit-test coverage anywhere in this codebase (jsdom has no real
 * HTML5 drag/pointer pipeline; see droppable-tab.tsx/TileLayout.core.tsx,
 * both untested at that layer for the same reason). This is the one piece
 * of the interaction's logic pure enough to verify directly.
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.2.
 */
export function dropPositionForPointerX(rect: Pick<DOMRect, "left" | "width">, clientX: number): "before" | "after" {
    return clientX < rect.left + rect.width / 2 ? "before" : "after";
}

/** Selector for the Pane header row that a header-hosted strip lives in —
 *  `BlockFrame_Header`'s own `data-role`, which is also what TileLayout
 *  already uses as the whole-pane drag handle. Matched by attribute rather
 *  than by class (`.block-frame-default-header`) deliberately: `data-role`
 *  is the stable contract blockframe.tsx exposes for exactly this kind of
 *  outside-in lookup, while the class name is styling. */
const PANE_HEADER_SELECTOR = '[data-role="block-header"]';

/**
 * Which element is the cross-pane drop zone for a strip — the whole Pane
 * header row when this strip is hosted in one (the normal case, via
 * PaneHeaderTabStrip), else the strip box itself.
 *
 * The drop zone is deliberately the ENTIRE header, not just the pills: a
 * pane with one short tab leaves most of its header as empty space, and
 * aiming at a ~100px strip to move a tab there is fussy — the whole row
 * reads as "this pane's tab area" to a user mid-drag. Widening it costs
 * nothing, because every drop target inside the header either accepts the
 * drag itself (the per-pill same-pane reorder targets) or declines and lets
 * it bubble here (pragmatic-dnd walks up the DOM on a false `canDrop`), and
 * the header's other chrome — ConnectionButton, EndIcons, the reserved §3.6
 * drag-handle spacer — registers no element drop target at all.
 *
 * Pure and exported for the same reason as `dropPositionForPointerX` above:
 * the gesture itself can't be unit-tested (no real drag pipeline in jsdom),
 * but *which element gets the registration* can — and getting that wrong is
 * silent, since pragmatic-dnd simply never fires a handler for an element
 * the pointer never reaches (cf. ReAgent's P0 on PR #3447, a cross-pane
 * move that was dead in the UI while its own unit tests passed).
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4.
 */
export function foreignDropRootFor(strip: HTMLElement): HTMLElement {
    return strip.closest<HTMLElement>(PANE_HEADER_SELECTOR) ?? strip;
}

export interface PaneTabStripProps<T> {
    tabs: T[];
    activeId: string | null;

    /** Content zoom for a strip that lives INSIDE a pane's content (the
     *  editor's file tabs), so it scales with that content. Pane-header
     *  tabs don't pass it: they scale only with chrome zoom, via the
     *  header row's own `--zoomfactor`. Omit for 1 (unzoomed). */
    zoomFactor?: Accessor<number>;

    /** Opt in to animating this strip's own shrink-to-fit width across a
     *  tab-count change (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md
     *  §2.4) instead of an instant snap. Only meaningful for a consumer
     *  that actually leaves the strip shrink-to-fit — the agent pane
     *  overrides it to a fixed `left:0;right:0` full-width box
     *  (SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md), where measuring
     *  and forcing an explicit `width` would fight that override (an
     *  explicit `left`+`width`+`right` all set is over-constrained — the
     *  browser drops `right`, un-stretching the strip for the animation's
     *  duration). Defaults to false: opt-in per consumer, not automatic. */
    animateWidth?: boolean;

    getId: (tab: T) => string;
    getLabel: (tab: T) => string;
    /** Optional icon rendered to the left of the label, inside a fixed-size
     *  `.pane-tab-icon` box. Pane headers always pass one (PaneChrome, via
     *  pane-tab-model.tsx); a strip that omits it reserves no space. */
    getIcon?: (tab: T) => JSX.Element;
    /** Full tooltip text; falls back to the label when omitted. */
    getTooltip?: (tab: T) => string;
    /** "Attention" tabs (unsaved changes, needs-review, …) always show
     *  their close × instead of only on hover. */
    getAttention?: (tab: T) => boolean;
    /** Extra classes beyond active/attention (e.g. an editor preview tab's
     *  italic label, a fork's running/idle status accent). */
    getTabClass?: (tab: T) => Record<string, boolean>;
    /** This tab's own pane color (PaneChrome only — every other consumer,
     *  editor file tabs and the agent History strip, omits it and gets
     *  zero behavior change). See `PaneTabColors`' own doc comment. */
    getColor?: (tab: T) => PaneTabColors | undefined;
    /** Pulse a pill when its tab's id is the source of an activity flash
     *  (the visual twin of a tool-call tone —
     *  docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md). Only
     *  meaningful where `getId` returns a blockId, i.e. a Pane's own header;
     *  editor file tabs and the agent History strip omit it. */
    flashOnActivity?: boolean;

    onActivate: (id: string) => void;
    /** Omit entirely (not just disable) to render tabs with no close ×. */
    onClose?: (id: string) => void;
    onTabDoubleClick?: (tab: T) => void;
    /** Custom label content (e.g. an inline "Save As" path input) instead
     *  of the plain text label. */
    renderLabel?: (tab: T) => JSX.Element;

    /** The far-right `+` — omitted entirely when the pane type has no
     *  "add tab" action. Always pinned last regardless of tab count or
     *  strip scroll state. */
    /** Optional MouseEvent param (universal Pane Tabs, PaneChrome) —
     *  lets a caller position a widget picker at the click. Every existing
     *  caller passes a zero-arg closure, which stays valid since the param
     *  is optional and simply goes unused there. */
    onAdd?: (e?: MouseEvent) => void;
    addTitle?: string;
    /** Visible text beside the `+` glyph, e.g. "New Agent". Opt-in per pane:
     *  omitted, the button stays the bare 28×28px glyph the editor and
     *  terminal strips use, so labelling one pane can't widen the others. */
    addLabel?: string;

    /** Opt in to reserving a small, non-scrolling-content spacer after the
     *  "+", present only once the strip is actually overflowing. Once a
     *  Pane's tab strip fills its whole header row, there is otherwise no
     *  space left in that row to grab for "drag the whole pane" — this
     *  spacer restores it, reachable by scrolling the strip to its end.
     *  Renders at zero width (and is skipped by the overflow measurement
     *  itself — see PaneTabStrip.scss) until overflow is real, so it costs
     *  nothing in the common case, where the header's own natural leftover
     *  space already serves this purpose. Deliberately NOT a drag target or
     *  handler of its own — it stays part of whichever ordinary "drag the
     *  whole pane" region already covers the header
     *  (see PaneHeaderTabStrip.tsx). Only meaningful for a strip used as a
     *  Pane's own header (PaneHeaderTabStrip); other consumers (the editor's
     *  file-tab strip, the agent History strip) render inside a Pane's
     *  content, not its header, and leave this off.
     *  SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.6. */
    reserveDragHandle?: boolean;

    /** Opt in to same-pane drag-reorder (Phase 3 of
     *  SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.2): `(blockId, targetId,
     *  position)` fires when a pill was dropped onto another pill in this
     *  SAME strip. Omitted entirely (the default) → no draggable()/
     *  dropTargetForElements() registered on any pill, zero behavior change
     *  for every existing consumer (editor file tabs, agent History strip)
     *  that doesn't pass it. Return `false` when the move was refused, so
     *  the pill doesn't play the landing bounce for a move that didn't
     *  happen; anything else (including `void`) counts as applied. */
    onReorder?: (blockId: string, targetId: string, position: "before" | "after") => boolean | void;

    /** This Pane's own stable identity (the caller's `nodeModel.nodeId`) —
     *  required whenever `onReorder` is passed. `paneTabItemType` is one
     *  module-level constant shared by EVERY `PaneTabStrip` instance in the
     *  window, so without this, a pill dragged from a DIFFERENT pane would
     *  pass `canDrop` (wrong-pane false affirmative: dimming/insertion-line
     *  feedback shown, then a silent no-op on drop, since
     *  `moveMemberInStack` correctly refuses a cross-pane move but nothing
     *  told the user beforehand). Rides in the drag payload as
     *  `sourceNodeId` and is checked against THIS strip's own `paneKey` in
     *  `canDrop` — cross-pane drops are Phase 4's job (§3.3), not silently
     *  half-supported here. ReAgent P1 on PR #3444. */
    paneKey?: string;

    /** Opt in to cross-pane drop-to-append (Phase 4 of
     *  SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4): fires with the
     *  dragged pill's `blockId` when a pill dragged from a DIFFERENT pane
     *  (a different `sourceNodeId`) is dropped anywhere on this pane's
     *  whole header row — including its empty space and its non-tab chrome,
     *  not just the pills (`foreignDropRootFor` resolves that element and
     *  documents why it's the entire row). Position within the row is
     *  irrelevant: a foreign tab always appends. Same-pane drops onto a
     *  specific pill are `onReorder`'s job instead.
     *  Registers a SEPARATE `dropTargetForElements` on that row, not a
     *  second one on any pill — pragmatic-dnd's registry
     *  is one-registration-per-DOM-element (confirmed via its source, not
     *  assumed), so this deliberately targets a different element than the
     *  per-pill ones `onReorder` uses, rather than risk clobbering them.
     *  Requires `paneKey`; omitted entirely (the default) → no
     *  registration, zero behavior change.
     *  Called one task AFTER the drop, not inside it (see the drop handler).
     *  Return `false` when the move was refused (same contract as
     *  `onReorder`). */
    onReceiveForeignTab?: (blockId: string) => boolean | void;
}

export function PaneTabStrip<T>(props: PaneTabStripProps<T>): JSX.Element {
    let stripRef: HTMLDivElement | undefined;
    let lastMeasuredWidth: number | undefined;
    let widthResetTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(widthResetTimeout));

    // `reserveDragHandle` (§3.6) — re-measured on every resize of the strip
    // itself (tab add/remove, pane resize, window resize, zoom change all
    // land here for free via ResizeObserver, unlike the width-transition
    // effect above which only reacts to `tabs.length`). The spacer starts at
    // zero width, so its own presence never feeds back into this
    // measurement until AFTER overflow is already real from the tabs alone.
    const [overflowing, setOverflowing] = createSignal(false);
    onMount(() => {
        const el = stripRef;
        if (!el || !props.reserveDragHandle) return;
        const measure = () => setOverflowing(el.scrollWidth > el.clientWidth);
        const ro = new ResizeObserver(measure);
        ro.observe(el);
        measure();
        onCleanup(() => ro.disconnect());
    });

    // Cross-pane drop-to-append (Phase 4, §3.4). A dwell-free hover flash —
    // unlike the outer Window Tab bar's spring-loaded switch (which needs a
    // dwell because committing reveals a HIDDEN tab), a target Pane's
    // content here is already visible the whole time, so there's nothing
    // to reveal before committing; see the spec's own reasoning for
    // skipping that dwell timer.
    const [foreignHover, setForeignHover] = createSignal(false);
    // Whether the hover highlight belongs on the strip box itself. False
    // once the drop zone resolves to the enclosing header row, so the
    // feedback outlines exactly the area that actually accepts the drop
    // rather than a sub-region of it. Only ever flips during onMount, i.e.
    // strictly before `foreignHover` can become true, so the JSX binding
    // below always reads a settled value.
    const [highlightStrip, setHighlightStrip] = createSignal(true);
    onMount(() => {
        const strip = stripRef;
        if (!strip || !props.onReceiveForeignTab || !props.paneKey) return;
        const paneKey = props.paneKey;
        // The whole header row, not just the strip — see
        // `foreignDropRootFor`'s own doc comment for why, and why the
        // widening is free with respect to the header's other chrome.
        const el = foreignDropRootFor(strip);
        if (el !== strip) {
            setHighlightStrip(false);
            // `el` is blockframe.tsx's element, outside this component's own
            // JSX, so the class goes on imperatively. Removed on cleanup:
            // the header's lifetime isn't tied to this strip's, and a stale
            // accent outline left on a header that outlives it would be
            // permanent.
            createEffect(() => el.classList.toggle("pane-header--foreign-hover", foreignHover()));
            onCleanup(() => el.classList.remove("pane-header--foreign-hover"));
        }
        const cleanup = dropTargetForElements({
            element: el,
            canDrop: ({ source }) =>
                source.data.type === paneTabItemType && source.data.sourceNodeId !== paneKey,
            onDragEnter: () => setForeignHover(true),
            onDragLeave: () => setForeignHover(false),
            onDrop: ({ source }) => {
                setForeignHover(false);
                const blockId = source.data.blockId as string | undefined;
                if (!blockId) return;
                // Commit on the next task, after pragmatic-dnd has finished
                // dispatching this drop. The move unmounts the dragged pill —
                // the live drag SOURCE — from its old strip (and, once moving
                // a pane's last tab closes that pane, its whole header, which
                // is itself a registered whole-pane draggable). Unmounting a
                // registered source mid-dispatch is the teardown hazard
                // SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md
                // catalogs; one task of delay costs nothing visible.
                // The hover styling above clears synchronously.
                // SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.2.
                const receive = props.onReceiveForeignTab!;
                setTimeout(() => {
                    if (receive(blockId) !== false) markLanded(blockId);
                }, 0);
            },
        });
        onCleanup(cleanup);
    });

    // FLIP-style width transition, opt-in via `animateWidth` (see that
    // prop's own doc comment for why it's opt-in, and PaneTabStrip.scss's
    // comment for why a plain CSS transition can't do this at all). Tracks
    // `tabs.length` specifically — that's the exact signal
    // `visibleTabs()`/`visibleTermTabs()` flip on (empty when there's
    // nothing to switch between, the full list once a 2nd tab exists),
    // matching §2.4's actual complaint (the strip's sudden appearance/
    // growth), not a general "animate on every possible width change"
    // feature.
    //
    // Measure AFTER each change (Solid's effects run after the DOM patch,
    // so `getBoundingClientRect()` here already reflects the NEW tab
    // count) and compare against whatever was measured on the PREVIOUS
    // run — that previous measurement naturally serves as "before" for
    // this change without needing to read the DOM pre-update at all. Then:
    // hold the box at the old width, force a synchronous reflow, and
    // transition to the new width — the standard FLIP technique. The
    // inline `width`/`transition` are cleared back to the CSS-driven
    // shrink-to-fit `auto` once the transition ends, so a later window
    // resize/zoom change isn't fighting a stale explicit pixel width.
    //
    // Deliberately NOT `{ defer: true }`: this effect must also run once
    // at mount, to record the initial width into `lastMeasuredWidth` with
    // no animation (nothing to animate FROM before mount). Deferring would
    // skip that first run, leaving `lastMeasuredWidth` unset going into the
    // very FIRST real tab-count change — exactly the 0-tabs-to-1-tab
    // transition §2.4 is about — silently skipping the one change this
    // feature exists to smooth, and only animating the second-and-later
    // ones. `lastMeasuredWidth !== undefined` below is what actually
    // distinguishes "first run, no prior measurement" from "no-op, sizes
    // matched" — not `on`'s own defer option.
    //
    // reagent's review of PR #2768: a second tabs.length change arriving
    // before the FIRST transition's cleanup timeout fires used to measure
    // `newWidth` while `el.style.width` was still pinned to the first
    // transition's in-flight interpolated value — reading garbage instead
    // of the true natural width for the current tab count, then holding at
    // a width unrelated to either state until the trailing timeout finally
    // cleared it. Fixed by clearing any still-pinned width/transition
    // FIRST (`wasAnimating` below) before measuring `newWidth`, and using
    // the box's actual current visual position (not the stale
    // `lastMeasuredWidth` target) as the hold point when interrupting an
    // in-flight transition — `lastMeasuredWidth` itself is only a valid
    // "before" reference for the SETTLED case, where layout is already
    // lazily dirty from Solid's DOM patch by the time this effect runs and
    // there's no other way to recover the pre-change size at all.
    createEffect(
        on(
            () => props.tabs.length,
            () => {
                if (!props.animateWidth) return;
                const el = stripRef;
                if (!el) return;
                const wasAnimating = widthResetTimeout !== undefined;
                const holdWidth = wasAnimating ? el.getBoundingClientRect().width : lastMeasuredWidth;
                clearTimeout(widthResetTimeout);
                el.style.transition = "none";
                el.style.width = "";
                // Forces the reflow that reveals the TRUE natural width for
                // the current tab count — only trustworthy once the line
                // above has cleared any lingering override.
                const newWidth = el.getBoundingClientRect().width;
                if (holdWidth !== undefined && holdWidth !== newWidth && !atoms.prefersReducedMotionAtom()) {
                    el.style.width = `${holdWidth}px`;
                    el.getBoundingClientRect(); // force reflow before the transition kicks in
                    el.style.transition = `width ${WIDTH_TRANSITION_MS}ms ease-out`;
                    el.style.width = `${newWidth}px`;
                    widthResetTimeout = setTimeout(() => {
                        el.style.width = "";
                        el.style.transition = "";
                    }, WIDTH_TRANSITION_MS + 20);
                }
                lastMeasuredWidth = newWidth;
            }
        )
    );

    // A plain vertical mouse wheel over a horizontally-scrolling region isn't
    // reliably redirected to horizontal scroll by the engine on its own —
    // explicit handling needed so "scroll the wheel over an overflowing tab
    // strip" actually works, the same affordance browsers' own native tab
    // bars give you. Only takes over when there's real horizontal overflow
    // AND the gesture is vertical (deltaY dominant) — a trackpad's own
    // horizontal swipe (deltaX dominant) is left to the browser's native
    // handling untouched, and a strip that isn't overflowing at all lets the
    // event bubble normally instead of silently swallowing every scroll.
    //
    // A real `addEventListener("wheel", ..., { passive: false })`, NOT the
    // JSX `onWheel` prop — Solid (like React) delegates common events
    // through a single top-level listener for perf, and delegated `wheel`
    // listeners are registered passive by default, which silently no-ops
    // `preventDefault()` (confirmed live: the JSX-prop version ran but
    // never actually scrolled). `{ passive: false }` here is what makes
    // `preventDefault()` real, so the browser's own default vertical-scroll
    // response to the wheel doesn't fight the manual `scrollLeft` write.
    onMount(() => {
        const el = stripRef;
        if (!el) return;
        const handleWheel = (e: WheelEvent) => {
            if (el.scrollWidth <= el.clientWidth) return;
            if (Math.abs(e.deltaX) >= Math.abs(e.deltaY)) return;
            e.preventDefault();
            el.scrollLeft += e.deltaY;
        };
        el.addEventListener("wheel", handleWheel, { passive: false });
        onCleanup(() => el.removeEventListener("wheel", handleWheel));
    });

    return (
        <div
            class="pane-tab-strip"
            classList={{ "pane-tab-strip--foreign-hover": foreignHover() && highlightStrip() }}
            ref={(el) => { stripRef = el; }}
            // Double-click inside the strip should never bubble up and
            // maximize the pane — matches the icon-toggle pattern from
            // blockframe.tsx. True for every consumer, not just the editor.
            onDblClick={(e) => e.stopPropagation()}
            // Set on the OUTER div (not inner) so it cascades down to both
            // this box's own `height` calc (PaneTabStrip.scss) and the
            // inner layer's `zoom` — a custom property set here is visible
            // to any descendant, which is all that's needed; the outer box
            // itself is deliberately never zoomed (§A.2 in the spec above).
            style={{ "--pane-tab-strip-zoom": String(props.zoomFactor?.() ?? 1) }}
        >
            {/* Zoom lives here, not on .pane-tab-strip itself — see
                docs/specs/SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md
                §A.2. The outer div stays real-pixel-sized (so an agent
                pane's edge-anchored `right: 0` never needs platform-
                specific zoom compensation); only this inner layer scales. */}
            <div class="pane-tab-strip-inner">
                <For each={props.tabs}>
                    {(tab) => (
                        <PaneTabStripItem
                            tab={tab}
                            active={props.activeId === props.getId(tab)}
                            getId={props.getId}
                            getLabel={props.getLabel}
                            getIcon={props.getIcon}
                            getTooltip={props.getTooltip}
                            getAttention={props.getAttention}
                            getTabClass={props.getTabClass}
                            getColor={props.getColor}
                            flashOnActivity={props.flashOnActivity}
                            onActivate={props.onActivate}
                            onClose={props.onClose}
                            onDoubleClick={props.onTabDoubleClick}
                            renderLabel={props.renderLabel}
                            onReorder={props.onReorder}
                            paneKey={props.paneKey}
                        />
                    )}
                </For>
                <Show when={props.onAdd}>
                    {/* Tooltip (Portal-based), same reason as the per-tab one
                        above: native `title` is slow/inconsistent in CEF.
                        delayMs={0} — instant, matching the status bar's own
                        tooltip feel (that one's a separate data-tip/Portal
                        system, but the same "no perceptible delay" intent). */}
                    <Tooltip divClassName="pane-tab-strip-add-tip" content={props.addTitle ?? "New tab"} delayMs={0}>
                        <button
                            type="button"
                            class={`pane-tab-strip-add${props.addLabel ? " pane-tab-strip-add-labeled" : ""}`}
                            aria-label={props.addLabel ?? props.addTitle ?? "New tab"}
                            onClick={(e) => props.onAdd!(e)}
                        >
                            {/* Wrapped so the glyph itself can be nudged (PaneTabStrip.scss's
                                .pane-tab-strip-add-glyph) without moving the button's own
                                box/hover-background/border — see that rule's comment. */}
                            <span class="pane-tab-strip-add-glyph">+</span>
                            <Show when={props.addLabel}>
                                <span class="pane-tab-strip-add-label">{props.addLabel}</span>
                            </Show>
                        </button>
                    </Tooltip>
                </Show>
                <Show when={props.reserveDragHandle}>
                    <div
                        class="pane-tab-strip-drag-handle"
                        classList={{ "pane-tab-strip-drag-handle--active": overflowing() }}
                    />
                </Show>
            </div>
        </div>
    );
}

interface PaneTabStripItemProps<T> {
    tab: T;
    active: boolean;
    getId: (tab: T) => string;
    getLabel: (tab: T) => string;
    getIcon?: (tab: T) => JSX.Element;
    getTooltip?: (tab: T) => string;
    getAttention?: (tab: T) => boolean;
    getTabClass?: (tab: T) => Record<string, boolean>;
    getColor?: (tab: T) => PaneTabColors | undefined;
    flashOnActivity?: boolean;
    onActivate: (id: string) => void;
    onClose?: (id: string) => void;
    onDoubleClick?: (tab: T) => void;
    renderLabel?: (tab: T) => JSX.Element;
    onReorder?: (blockId: string, targetId: string, position: "before" | "after") => boolean | void;
    paneKey?: string;
}

function PaneTabStripItem<T>(props: PaneTabStripItemProps<T>): JSX.Element {
    const id = () => props.getId(props.tab);
    const attention = () => props.getAttention?.(props.tab) ?? false;
    let pillRef: HTMLDivElement | undefined;
    const [isDragging, setIsDragging] = createSignal(false);
    const [dropSide, setDropSide] = createSignal<"before" | "after" | null>(null);

    // Activity flash: click this pill on every tone from its own block,
    // whether or not its window tab is showing. The color comes from the
    // pill's own `--pane-tab-underline` in the stylesheet, so no base
    // color is passed here.
    onMount(() => {
        if (!props.flashOnActivity) return;
        const unsubscribe = onActivityFlash(({ blockId }) => {
            if (blockId === id() && pillRef) flashElement(pillRef);
        });
        onCleanup(unsubscribe);
    });

    // Same-pane drag-reorder (Phase 3, SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
    // §3.1/§3.2). Gated entirely on `onReorder` being passed — every existing
    // consumer that doesn't (editor file tabs, agent History strip) gets no
    // draggable()/dropTargetForElements() registration at all, zero behavior
    // change. A pill is simultaneously a drag SOURCE (itself) and a drop
    // TARGET (for another pill being dragged onto it) — same dual-role
    // pattern droppable-tab.tsx's own tab buttons already use for the outer
    // Window Tab bar.
    onMount(() => {
        if (!props.onReorder) return;
        const el = pillRef;
        if (!el) return;
        const cleanupDraggable = draggable({
            element: el,
            getInitialData: () => ({
                kind: "pane-tab",
                blockId: id(),
                type: paneTabItemType,
                sourceNodeId: props.paneKey,
            }),
            onDragStart: () => setIsDragging(true),
            onDrop: () => setIsDragging(false),
        });
        const cleanupDropTarget = dropTargetForElements({
            element: el,
            // Same-pane only — a pill dragged from a DIFFERENT pane (a
            // different `sourceNodeId`) is rejected here rather than
            // accepted-then-silently-no-op'd on drop. Cross-pane drop is
            // Phase 4 (§3.3), not yet implemented. ReAgent P1 on PR #3444.
            canDrop: ({ source }) =>
                source.data.type === paneTabItemType &&
                source.data.blockId !== id() &&
                source.data.sourceNodeId === props.paneKey,
            onDrag: ({ location }) => {
                setDropSide(dropPositionForPointerX(el.getBoundingClientRect(), location.current.input.clientX));
            },
            onDragLeave: () => setDropSide(null),
            onDrop: ({ source, location }) => {
                setDropSide(null);
                const blockId = source.data.blockId as string | undefined;
                if (!blockId) return;
                // Computed fresh here, not reused from the `onDrag`-updated
                // signal above — avoids relying on that signal's last value
                // still being current at the exact moment of drop.
                const position = dropPositionForPointerX(el.getBoundingClientRect(), location.current.input.clientX);
                // The moved pill (not the one it was dropped on) bounces,
                // same as a reordered Window Tab.
                if (props.onReorder!(blockId, id(), position) !== false) markLanded(blockId);
            },
        });
        onCleanup(() => {
            cleanupDraggable();
            cleanupDropTarget();
        });
    });

    const onMouseDown = (e: MouseEvent) => {
        // Middle-click → close (matches VS Code / Chrome convention).
        if (e.button === 1) {
            e.preventDefault();
            props.onClose?.(id());
        }
    };

    const onClick = (e: MouseEvent) => {
        // Ignore middle-click here — onMouseDown already handled it.
        if (e.button !== 0) return;
        if (!props.active) props.onActivate(id());
    };

    const onDblClick = (e: MouseEvent) => {
        e.stopPropagation();
        props.onDoubleClick?.(props.tab);
    };

    const onCloseClick = (e: MouseEvent) => {
        e.stopPropagation();
        props.onClose?.(id());
    };

    // Applies to every pill, active included: `--pane-tab-bg` is what both
    // the resting rule and `&--active` resolve, so a selected tab paints
    // its own color too and only falls back to the plain
    // `--block-bg-color` when it has none
    // (SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md §2.2). Read via a CSS
    // custom property rather than a direct inline `background-color` so the
    // stylesheet keeps control of precedence across the resting / hover /
    // active rules, same pattern as tab.tsx's `--tab-color`.
    const colorStyle = (): JSX.CSSProperties => {
        const c = props.getColor?.(props.tab);
        if (!c) return {};
        return {
            ...(c.background ? { "--pane-tab-bg": c.background } : {}),
            ...(c.underline ? { "--pane-tab-underline": c.underline } : {}),
            ...(c.neutralBackground ? { "--pane-tab-neutral-bg": c.neutralBackground } : {}),
        } as JSX.CSSProperties;
    };

    // Tooltip (Portal-based) rather than native `title` — the strip has
    // overflow:hidden, which would clip a CSS tooltip, and native `title`
    // is slow/inconsistent in CEF. The Tooltip's wrapper div carries the
    // flex sizing (.pane-tab-tip); the tab keeps its own mousedown/click/
    // dblclick handlers.
    return (
        <Tooltip
            placement="bottom"
            divClassName="pane-tab-tip"
            content={props.getTooltip?.(props.tab) ?? props.getLabel(props.tab)}
        >
            <div
                class="pane-tab"
                classList={{
                    "pane-tab--active": props.active,
                    "pane-tab--attention": attention(),
                    "pane-tab--dragging": isDragging(),
                    "pane-tab--drop-before": dropSide() === "before",
                    "pane-tab--drop-after": dropSide() === "after",
                    "pane-tab--landing": landingTabId() === id(),
                    // Gates PaneTabStrip.scss's lighter-on-hover treatment —
                    // only a tab with its own color gets it; every other
                    // consumer's plain hover tint is untouched.
                    "pane-tab--colored": !!props.getColor?.(props.tab)?.background,
                    ...(props.getTabClass?.(props.tab) ?? {}),
                }}
                style={colorStyle()}
                ref={(el) => { pillRef = el; }}
                onMouseDown={onMouseDown}
                onClick={onClick}
                onDblClick={onDblClick}
            >
                {props.getIcon && <span class="pane-tab-icon">{props.getIcon(props.tab)}</span>}
                {props.renderLabel ? (
                    props.renderLabel(props.tab)
                ) : (
                    <span class="pane-tab-label">{props.getLabel(props.tab)}</span>
                )}
                <Show when={props.onClose}>
                    <button
                        class="pane-tab-close"
                        onClick={onCloseClick}
                        title={attention() ? "Close (unsaved changes)" : "Close"}
                        aria-label="Close tab"
                    >
                        {/* Always in the DOM for attention tabs (closing
                            those may need confirmation); hover-shown
                            otherwise, purely via CSS opacity. */}
                        ×
                    </button>
                </Show>
            </div>
        </Tooltip>
    );
}
