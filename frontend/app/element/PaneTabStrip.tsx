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
import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { markEscaped } from "@/app/drag/drag-session";
import {
    isDraggedDocTab,
    isDraggedPaneTab,
    releaseDocTabDrag,
    releasePaneTabDrag,
    startDocTabDrag,
    startPaneTabDrag,
} from "@/app/drag/pane-tab-drag";
import { flashElement, onActivityFlash } from "@/app/notification/activity-flash";
import { atoms } from "@/store/global";
import { isWindows } from "@/util/platformutil";
import { Tooltip } from "./tooltip";
import "./PaneTabStrip.scss";
// The Pane Tab pill's drag tag (drag-types.ts): distinct from the tile and
// window-tab tags, so no existing target mistakes a pill for either.
// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.1.
import { docTabItemType, paneTabItemType } from "@/app/drag/drag-types";

// Matches the other reveal-gate/cross-fade durations added alongside this
// one in SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.4.
const WIDTH_TRANSITION_MS = 160;


/** How long a just-landed pill keeps `.pane-tab--landing` — the Window Tab
 *  bar's own clear-timeout for its bounce (tab-reorder.ts), so the two match. */
export const LANDING_BOUNCE_MS = 400;

/** The tab that was just dropped into place and should play the landing
 *  bounce: its id AND the pane (`paneKey`) it landed in. Module-level rather
 *  than per-strip: a cross-pane drop re-renders the pill in the destination
 *  strip (mounting it fresh there), so the flag has to be readable by
 *  whichever strip ends up rendering it. The pane is part of the identity
 *  because one block can have pills in several strips: agent fork lineages
 *  show blocks from OTHER panes as extra tabs (`PaneChromeModel.extraTabs`),
 *  and only the pill in the pane it actually landed in should bounce (Codex
 *  P2 on #3694). Only header strips set it.
 *  SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §3.4. */
const [landedTab, setLandedTab] = createSignal<{ id: string; paneKey: string | undefined } | null>(null);
let landingTimer: ReturnType<typeof setTimeout> | undefined;
function markLanded(id: string, paneKey: string | undefined): void {
    clearTimeout(landingTimer);
    setLandedTab({ id, paneKey });
    landingTimer = setTimeout(() => setLandedTab(null), LANDING_BOUNCE_MS);
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
    /** The selected pill's background (a step stronger than `background`);
     *  falls back to `background` when absent. */
    activeBackground?: string;
    neutralBackground?: string;
}

/**
 * Document-tab drag for a strip of a pane's documents (an Editor's files, a
 * Media pane's files): the same pills, drop marks and landing bounce as pane
 * tabs, but its own drag kind. A document tab moves only within its strip or
 * to another pane of the same `docType`, and never tears off into a window.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md section 3.1.
 */
export interface DocTabDrag {
    /** The pane type ("editor", "media"): tabs move only between panes of one type. */
    docType: string;
    /** The block whose tabs these are. */
    blockId: string;
    /** Whether a tab can be dragged at all (an empty Media tab can not). */
    canDrag?: (tabId: string) => boolean;
    /** A tab of this strip dropped before or after another of its tabs.
     *  `false`: refused (no landing bounce). */
    onReorder: (tabId: string, targetId: string, position: "before" | "after") => boolean | void;
    /** A tab of another pane of this type dropped on one of these tabs. Runs
     *  one task after the drop: the move unmounts the dragged pill, the live
     *  source of the drag. Omitted: such a drop is not accepted here. */
    onReceive?: (sourceBlockId: string, tabId: string, at: { targetId: string; position: "before" | "after" }) => boolean | void;
}

/** What a document-tab drag carries (pragmatic-dnd's `source.data`). */
export interface DocTabDragData {
    type: typeof docTabItemType;
    tabId: string;
    docType: string;
    sourceBlockId: string;
}

export function asDocTabDragData(data: Record<string | symbol, unknown>): DocTabDragData | null {
    return data.type === docTabItemType &&
        typeof data.tabId === "string" &&
        typeof data.docType === "string" &&
        typeof data.sourceBlockId === "string"
        ? (data as unknown as DocTabDragData)
        : null;
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

/** Selector for a whole Pane — PaneChrome's root (`.pane-stack`), header
 *  and body together. Matched by `data-role` for the same reason as the
 *  header selector above: it's the stable contract, the class is styling. */
const PANE_SELECTOR = '[data-role="pane"]';

/**
 * Which element is the cross-pane drop zone for a strip: the WHOLE Pane
 * (PaneChrome's root, header and body together) when the strip is that
 * pane's header strip, else the header row (a header outside PaneChrome),
 * else the strip box itself (a strip that isn't in a header at all).
 *
 * Dropping a Pane Tab anywhere on another pane, header or body, does the same
 * thing: the tab joins that pane (repo owner, 2026-09-24,
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.3). It was the header row alone
 * before, and before that just the pills: aiming at a thin strip to move a tab
 * there is fussy, and the whole pane is what reads as "into this pane" mid-drag.
 * Widening it is free: every drop target inside the pane either accepts the
 * drag itself (the per-pill same-pane reorder targets) or declines and lets it
 * bubble here (pragmatic-dnd walks up the DOM on a false `canDrop`). Nothing
 * else inside a pane registers an element drop target, and the tile overlay
 * that sits above pane bodies only takes pointer events during a whole-PANE
 * drag (`layoutModel.activeDrag`), never during a pane-TAB drag.
 *
 * Pure and exported for the same reason as `dropPositionForPointerX` above:
 * the gesture itself can't be unit-tested (no real drag pipeline in jsdom),
 * but *which element gets the registration* can — and getting that wrong is
 * silent, since pragmatic-dnd simply never fires a handler for an element
 * the pointer never reaches (cf. ReAgent's P0 on PR #3447, a cross-pane
 * move that was dead in the UI while its own unit tests passed).
 */
export function foreignDropRootFor(strip: HTMLElement): HTMLElement {
    // Widen to the pane only from a strip that IS the pane's header strip. A
    // strip rendered inside a pane's content (editor file tabs, the agent
    // History strip) must never resolve to the pane around it.
    const header = strip.closest<HTMLElement>(PANE_HEADER_SELECTOR);
    if (!header) return strip;
    return header.closest<HTMLElement>(PANE_SELECTOR) ?? header;
}

/**
 * Which element shows the "valid drop target" flash while a foreign tab
 * hovers the drop zone: the header row, wherever over the pane the pointer
 * is. The header is where the tab will appear, and the flash is the Window
 * Tab look, which is sized for a strip, not a whole pane body
 * (SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §3).
 * Falls back to the strip box for a strip not hosted in a header.
 */
export function foreignDropHighlightFor(strip: HTMLElement): HTMLElement {
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
    /**
     * A second, wrapping line under the label (an agent's summary). When it returns
     * text the tooltip shows the label and this line, and appears IMMEDIATELY (no
     * hover delay, no fade): it is read at a glance, not waited for. When it returns
     * nothing the tab's tooltip is exactly what it was.
     */
    getTooltipDetail?: (tab: T) => string | undefined;
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
     *  (a different `sourceNodeId`) is dropped anywhere on this pane —
     *  header or body, empty space and non-tab chrome included
     *  (`foreignDropRootFor` resolves that element and documents why it's
     *  the whole pane). Where on the pane is irrelevant: a foreign tab
     *  always appends. Same-pane drops onto a specific pill are
     *  `onReorder`'s job instead.
     *  Registers a SEPARATE `dropTargetForElements` on that element, not a
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

    /** The window tab this pane lives in. When set (with `onReorder` and
     *  `paneKey`), a pill dragged OUT of the window tears off into a floating
     *  pane: its drag carries a `"pane-tab"` cross-window payload that the
     *  `CrossWindowDragMonitor`s act on (pane-tab-tearoff.ts). Omitted → pills
     *  only reorder / move between panes, as before.
     *  SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.5. */
    sourceTabId?: string;

    /** Make the pills document tabs (`DocTabDrag`) instead of pane tabs:
     *  their own drag kind, no tear-off. Takes the place of `onReorder`,
     *  `paneKey`, `onReceiveForeignTab` and `sourceTabId`, which are for pane
     *  tabs. */
    docDrag?: DocTabDrag;
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
        // Read on the next frame, not in the observer callback: there, other
        // observers have already written to the DOM, so each strip's read
        // forced its own layout on every frame of a window drag.
        let frame: number | undefined;
        const ro = new ResizeObserver(() => {
            if (frame == null) frame = requestAnimationFrame(() => { frame = undefined; measure(); });
        });
        ro.observe(el);
        measure();
        onCleanup(() => { ro.disconnect(); if (frame != null) cancelAnimationFrame(frame); });
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
        // The whole pane, header and body — see `foreignDropRootFor`'s own
        // doc comment for why, and why the widening is free.
        const el = foreignDropRootFor(strip);
        // The flash goes on the header either way (`foreignDropHighlightFor`).
        const highlightEl = foreignDropHighlightFor(strip);
        if (highlightEl !== strip) {
            setHighlightStrip(false);
            // `highlightEl` is blockframe.tsx's element, outside this
            // component's own JSX, so the class goes on imperatively. Removed
            // on cleanup: the header's lifetime isn't tied to this strip's,
            // and a stale accent outline left on a header that outlives it
            // would be permanent.
            createEffect(() => highlightEl.classList.toggle("pane-header--foreign-hover", foreignHover()));
            onCleanup(() => highlightEl.classList.remove("pane-header--foreign-hover"));
        }
        const cleanup = dropTargetForElements({
            element: el,
            canDrop: ({ source }) =>
                source.data.type === paneTabItemType && source.data.sourceNodeId !== paneKey,
            onDragEnter: () => setForeignHover(true),
            onDragLeave: () => setForeignHover(false),
            onDrop: ({ source }) => {
                setForeignHover(false);
                // Handled in-window: the cross-window monitor must not also
                // tear this tab off on dragend (pane-tab-tearoff.ts).
                setCurrentDragPayload(null);
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
                    if (receive(blockId) !== false) markLanded(blockId, paneKey);
                }, 0);
            },
        });
        onCleanup(cleanup);
    });

    // FLIP-style width transition, opt-in via `animateWidth` (see that
    // prop's own doc comment for why it's opt-in, and PaneTabStrip.scss's
    // comment for why a plain CSS transition can't do this at all). Tracks
    // `tabs.length` specifically — a tab added or removed is what makes the
    // strip suddenly appear or grow, matching §2.4's actual complaint, not
    // a general "animate on every possible width change" feature.
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
                            getTooltipDetail={props.getTooltipDetail}
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
                            sourceTabId={props.sourceTabId}
                            docDrag={props.docDrag}
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
    getTooltipDetail?: (tab: T) => string | undefined;
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
    sourceTabId?: string;
    docDrag?: DocTabDrag;
}

/** How one pill drags, for either kind of tab: what it carries, what it
 *  accepts, and what a drop on it does. One registration (below) serves both. */
interface PillDrag {
    data: () => Record<string, unknown>;
    canDrag: () => boolean;
    start: () => void;
    release: () => void;
    accepts: (data: Record<string | symbol, unknown>) => boolean;
    /** Act on a drop at `position`; returns the id of the tab to bounce, or null. */
    drop: (data: Record<string | symbol, unknown>, position: "before" | "after") => string | null;
}

function PaneTabStripItem<T>(props: PaneTabStripItemProps<T>): JSX.Element {
    const id = () => props.getId(props.tab);
    const attention = () => props.getAttention?.(props.tab) ?? false;
    let pillRef: HTMLDivElement | undefined;
    // The pane a landing bounce is for: the block, for document tabs.
    const landingKey = () => props.docDrag?.blockId ?? props.paneKey;
    const isDragging = () =>
        props.docDrag ? isDraggedDocTab(props.docDrag.blockId, id()) : isDraggedPaneTab(id(), props.paneKey);
    const [dropSide, setDropSide] = createSignal<"before" | "after" | null>(null);

    // Activity flash: click this pill on every tone from its own block,
    // whether or not its window tab is showing. The color comes from the
    // pill's own `--pane-tab-underline` in the stylesheet, so no base
    // color is passed here.
    onMount(() => {
        if (!props.flashOnActivity) return;
        const unsubscribe = onActivityFlash((target) => {
            if (target.blockId === id() && pillRef) flashElement(pillRef, target);
        });
        onCleanup(unsubscribe);
    });

    // Tear-off to a floating pane (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
    // §3.5). On drag start, hand the cross-window monitors a "pane-tab"
    // payload; they tear the tab off on dragend only if no drop target inside
    // the window claimed it (both pill drop targets clear it) and it landed on
    // no AgentMux window. The payload is deliberately NOT cleared in the
    // draggable's own onDrop: dragend, which the monitors read it on, comes
    // after — same as whole-pane (tile) drags.
    const onEscape = (e: KeyboardEvent) => {
        // keydown still reaches the page during an HTML5 drag; an escaped
        // drag never tears off (the monitors read it off the session).
        if (e.key === "Escape") markEscaped();
    };
    let tracking = false;
    const startTearOffTracking = () => {
        if (!props.paneKey || !props.sourceTabId || !pillRef) return;
        const paneRect = pillRef.closest<HTMLElement>('[data-role="pane"]')?.getBoundingClientRect();
        setCurrentDragPayload({
            kind: "pane-tab",
            blockId: id(),
            sourceNodeId: props.paneKey,
            sourceTabId: props.sourceTabId,
            paneSize: paneRect ? { width: paneRect.width, height: paneRect.height } : undefined,
        });
        // macOS/Linux: without this, a drop outside every drop target (i.e. a
        // tear-off) animates the drag image back into the window first, the
        // same snapback whole-pane drags suppress (TileLayout.darwin.tsx).
        if (!isWindows()) preventUnhandled.start();
        window.addEventListener("keydown", onEscape, true);
        tracking = true;
    };
    const stopTearOffTracking = () => {
        if (!tracking) return;
        tracking = false;
        if (!isWindows()) preventUnhandled.stop();
        window.removeEventListener("keydown", onEscape, true);
    };
    onCleanup(stopTearOffTracking);

    // A pane tab: same-pane reorder (Phase 3,
    // SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md section 3.1 and 3.2), gated on
    // `onReorder`, with tear-off when the strip names its window tab (3.5). A
    // pill from a DIFFERENT pane (another `sourceNodeId`) is rejected here
    // rather than accepted and then silently ignored on drop: cross-pane is
    // the whole-pane target's job (#3444).
    const paneTabDrag = (): PillDrag | null => {
        const onReorder = props.onReorder;
        if (!onReorder) return null;
        return {
            data: () => ({ kind: "pane-tab", blockId: id(), type: paneTabItemType, sourceNodeId: props.paneKey }),
            canDrag: () => true,
            start: () => {
                startPaneTabDrag(id(), props.paneKey, props.sourceTabId);
                startTearOffTracking();
            },
            release: () => {
                releasePaneTabDrag();
                stopTearOffTracking();
            },
            accepts: (data) =>
                data.type === paneTabItemType && data.blockId !== id() && data.sourceNodeId === props.paneKey,
            drop: (data, position) => {
                // Handled in-window: never also a tear-off.
                setCurrentDragPayload(null);
                const blockId = data.blockId as string | undefined;
                if (!blockId) return null;
                return onReorder(blockId, id(), position) !== false ? blockId : null;
            },
        };
    };

    // A document tab (`docDrag`): reorder within the strip, or a tab of
    // another pane of the same type dropped onto this one. Never a tear-off.
    const docTabDrag = (): PillDrag | null => {
        const doc = props.docDrag;
        if (!doc) return null;
        return {
            data: () => ({ type: docTabItemType, tabId: id(), docType: doc.docType, sourceBlockId: doc.blockId }) satisfies DocTabDragData,
            canDrag: () => doc.canDrag?.(id()) ?? true,
            start: () => startDocTabDrag(doc.blockId, id()),
            release: releaseDocTabDrag,
            accepts: (raw) => {
                const data = asDocTabDragData(raw);
                if (!data || data.docType !== doc.docType) return false;
                return data.sourceBlockId === doc.blockId ? data.tabId !== id() : !!doc.onReceive;
            },
            drop: (raw, position) => {
                const data = asDocTabDragData(raw);
                if (!data) return null;
                if (data.sourceBlockId === doc.blockId) {
                    return doc.onReorder(data.tabId, id(), position) !== false ? data.tabId : null;
                }
                // From another pane: commit on the next task, after
                // pragmatic-dnd has finished dispatching this drop, because
                // the move unmounts the dragged pill, the live source of the
                // drag (SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md 4.2).
                const receive = doc.onReceive!;
                const at = { targetId: id(), position };
                setTimeout(() => {
                    if (receive(data.sourceBlockId, data.tabId, at) !== false) markLanded(data.tabId, doc.blockId);
                }, 0);
                return null;
            },
        };
    };

    // A pill is both a drag source (itself) and a drop target (for another
    // pill dragged onto it), the same dual role droppable-tab.tsx's tab
    // buttons have in the window tab bar. A strip with neither `onReorder`
    // nor `docDrag` registers nothing.
    onMount(() => {
        const drag = docTabDrag() ?? paneTabDrag();
        const el = pillRef;
        if (!drag || !el) return;
        const cleanupDraggable = draggable({
            element: el,
            canDrag: () => drag.canDrag(),
            getInitialData: () => drag.data(),
            onDragStart: () => drag.start(),
            onDrop: () => drag.release(),
        });
        const cleanupDropTarget = dropTargetForElements({
            element: el,
            canDrop: ({ source }) => drag.accepts(source.data),
            onDrag: ({ location }) => {
                setDropSide(dropPositionForPointerX(el.getBoundingClientRect(), location.current.input.clientX));
            },
            onDragLeave: () => setDropSide(null),
            onDrop: ({ source, location }) => {
                setDropSide(null);
                // Computed fresh here, not reused from the `onDrag`-updated
                // signal above: that value may not be current at the drop.
                const position = dropPositionForPointerX(el.getBoundingClientRect(), location.current.input.clientX);
                // The moved pill (not the one it was dropped on) bounces,
                // same as a reordered Window Tab.
                const landed = drag.drop(source.data, position);
                if (landed) markLanded(landed, landingKey());
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
    // active rules, same pattern as tab.tsx's `--tab-bg` / `--tab-bg-active` / `--tab-underline`.
    const colorStyle = (): JSX.CSSProperties => {
        const c = props.getColor?.(props.tab);
        if (!c) return {};
        return {
            ...(c.background ? { "--pane-tab-bg": c.background } : {}),
            ...(c.activeBackground ? { "--pane-tab-bg-active": c.activeBackground } : {}),
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
            immediate={!!props.getTooltipDetail?.(props.tab)}
            content={
                props.getTooltipDetail?.(props.tab) ? (
                    <div class="pane-tab-tip-body" data-testid="pane-tab-tip-body">
                        <div class="pane-tab-tip-label">{props.getTooltip?.(props.tab) ?? props.getLabel(props.tab)}</div>
                        <div class="pane-tab-tip-summary">{props.getTooltipDetail?.(props.tab)}</div>
                    </div>
                ) : (
                    (props.getTooltip?.(props.tab) ?? props.getLabel(props.tab))
                )
            }
        >
            <div
                class="pane-tab"
                classList={{
                    "pane-tab--active": props.active,
                    "pane-tab--attention": attention(),
                    "pane-tab--dragging": isDragging(),
                    "pane-tab--drop-before": dropSide() === "before",
                    "pane-tab--drop-after": dropSide() === "after",
                    "pane-tab--landing": landedTab()?.id === id() && landedTab()?.paneKey === landingKey(),
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
