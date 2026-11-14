// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneTabStrip — the shared, pane-type-agnostic tab strip. Extracted from
 * the editor's first tab strip (`.editor-tab-strip`/`.editor-tab`) so
 * agent-pane forks and terminal-pane shell tabs can reuse the exact same
 * chrome and interaction model instead of each pane type growing its own
 * copy. Document tabs (Editor, Media) draw it through DocTabStrip.tsx.
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

import { landedTab, markLanded } from "./pane-tab-landing";
export { LANDING_BOUNCE_MS } from "./pane-tab-landing";
import { createEffect, For, on, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";
import { isDraggedDocTab, isDraggedPaneTab } from "@/app/drag/pane-tab-drag";
import { shouldReduceMotion } from "@/app/motion/motion";
import { usePulseWhen } from "@/app/motion/solid";
import { Tooltip } from "./tooltip";
import { CloseButton } from "./ui/CloseButton";
import { tip } from "./tip";
import "./PaneTabStrip.scss";
import { createScrollEdges } from "./scroll-edges";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import type { PaneTabSelectionApi } from "./pane-tab-selection";
import { createStripReveal, registerStripWithPane } from "./strip-reveal-controller";
import { STRIP_EDGE_FADE_PX } from "./strip-reveal";
import { outerStripScroller } from "./strip-scroller";
import { foreignDropHighlightFor, foreignDropRootFor, incomingTabLabel, useForeignTabDrop, type PaneTabBodyDrop } from "./pane-tab-foreign-drop";
import { useStripScrollInput, useStripWidthAnimation } from "./pane-tab-strip-hooks";
import { dropPositionForPointerX, pillTooltipContent, usePillDrag, usePillFlash, usePillGestures, usePillProgress, type DocTabDrag } from "./pane-tab-pill";

export { dropPositionForPointerX, foreignDropHighlightFor, foreignDropRootFor, incomingTabLabel };
export type { DocTabDrag };
import type { PaneTabProgress } from "@/app/store/pane-tab-progress";




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
    /**
     * A line between the label and the detail: an agent's seat ("Leader · product —
     * reports to the operator — agent pane, identity"). Shown, immediately, like the
     * detail, with or without one (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4).
     */
    getTooltipSeat?: (tab: T) => string | undefined;
    /** An agent tab's harness and the model answering, as a line of its tooltip. */
    getTooltipModel?: (tab: T) => string | undefined;
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
    /** What a pill shows about its tab's work: a line along its top (SPEC_PANE_TAB_PROGRESS_LINE_2026_10_24.md).
     *  Called once per pill, in its component scope, so it may subscribe and clean up
     *  there. Omitted by strips whose tabs are not agents. */
    useProgress?: (tab: T) => Accessor<PaneTabProgress | null>;
    /** The pane is collapsed. The selected pill of an expanded pane shows no line (its
     *  pane's border and transcript already say it); in a collapsed pane it does. */
    collapsed?: Accessor<boolean>;
    /** This tab's close is under way: its pill dims and shows a spinner in place of its ×. */
    isClosing?: (tab: T) => boolean;
    /** A right-click on a pill. The handler decides whether there is a menu to show. */
    onTabContextMenu?: (tab: T, e: MouseEvent) => void;

    onActivate: (id: string) => void;
    /** A plain left-click on a pill, active or not, before `onActivate` (which a
     *  click on the active pill never reaches). A Pane's header uses it to
     *  expand a collapsed pane. Omitted by strips that have no such pane. */
    onSelect?: (id: string) => void;
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
    onReceiveForeignTab?: (blockId: string, opts?: { keepSeparate?: boolean }) => boolean | void;
    /** Several tabs dragged together (Ctrl/Cmd+click selection) dropped on this pane:
     *  `blockIds` is the whole drag, `grabbed` the pill the pointer held. Returns the
     *  blocks that moved (they bounce), or `false` when nothing could be moved. Called
     *  one task after the drop, like `onReceiveForeignTab`.
     *  SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.3. */
    onReceiveForeignTabs?: (
        blockIds: readonly string[],
        grabbed: string,
        opts?: { keepSeparate?: boolean }
    ) => readonly string[] | false;
    /** Whether this pane would take a multi-tab drag: true when at least one of
     *  `blockIds` is not already in it. Without it a multi-tab drag is accepted only
     *  from another pane, as a single tab is. */
    acceptsForeignTabs?: (blockIds: readonly string[]) => boolean;
    /** Opt in to placing a single pill dropped on this pane's body as a pane of its own. The
     *  header and the middle of the body still join the pill to this pane's tabs; the rest of the
     *  body shows the placement ghost (as when a whole pane is dragged) and `place` splits the
     *  tab out on the side the ghost shows. `place` returns the new pane's key (the pill bounces
     *  there), or `false` when it was refused. Without it a drop anywhere on the pane joins, as
     *  before. A multi-tab drag always joins.
     *  SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.1. */
    bodyDrop?: PaneTabBodyDrop;
    /** Ctrl/Cmd+click multi-select for a Pane's own header strip. Omitted by document
     *  tabs and the agent History strip, which have no selection. */
    selection?: PaneTabSelectionApi;

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

/** What a strip keeps for itself: its element, the scroll fades and the reveal of its selected tab. */
function createStripState<T>(props: PaneTabStripProps<T>) {
    const refs: { strip?: HTMLDivElement } = {};

    // Edge fades while the tabs overflow (scroll-edge-fade, PaneTabStrip.scss).
    // The selected tab sitting flush at the left edge drops the left fade, so it is not dimmed.
    const edges = createScrollEdges(
        () => refs.strip,
        () => {
            const id = props.activeId;
            if (id == null || !refs.strip) return undefined;
            for (const el of refs.strip.querySelectorAll<HTMLElement>("[data-tab-id]")) if (el.dataset.tabId === id) return el;
            return undefined;
        }
    );

    // Keeps the selected tab in view (strip-reveal-controller.ts). A strip in a
    // hidden window tab, or in a kept-alive document block that isn't showing,
    // reads no geometry; it reveals once when it is shown again.
    const windowDisplayed = useWindowTabDisplayed();
    const blockVisibility = props.docDrag ? usePaneTabVisibility(props.docDrag.blockId) : undefined;
    // In a pane's header the strip does not scroll itself: the header's range scrolls it, with the header's
    // controls after it (strip-scroller.ts), so the reveal scrolls that box and keeps clear of its fade.
    const outer = () => outerStripScroller(refs.strip);
    const scrollBox = () => outer()?.el ?? refs.strip;
    const reveal = createStripReveal({
        strip: scrollBox,
        activeId: () => props.activeId,
        tabIds: () => props.tabs.map(props.getId),
        shown: () => windowDisplayed() && (blockVisibility?.() ?? "active") === "active",
        overflows: () => outer()?.scroller.overflows() ?? (edges.moreLeft() || edges.moreRight()),
        edgeClearance: () => outer()?.scroller.edgeFadePx ?? STRIP_EDGE_FADE_PX,
        lastTabEndsStrip: () => !outer(),
    });
    registerStripWithPane(scrollBox, reveal);
    onMount(() => {
        const box = outer();
        if (!box) return;
        onCleanup(box.scroller.onUserScroll(reveal.noteUserScroll));
        // The box starts where the pane's last header was: the tab the person picked travels from there.
        onCleanup(box.scroller.onCarried(() => reveal.request("selected")));
    });
    // A new selection may leave the strip where it was (already flush, or hidden until shown): measure the fade again.
    const shown = () => windowDisplayed() && (blockVisibility?.() ?? "active") === "active";
    createEffect(
        on([() => props.activeId, shown], () => {
            if (shown()) edges.remeasure();
        }, { defer: true })
    );
    return { refs, edges, reveal };
}
type StripState = ReturnType<typeof createStripState>;

// A document tab in front closed (×, Ctrl+W, a menu, moved away): the
// tab that takes its place plays the landing cue, so the change shows even
// when the tabs around it read the same. Document strips only.
function trackLanding<T>(props: PaneTabStripProps<T>): void {
    createEffect(
        on(
            () => [props.activeId, props.tabs.map(props.getId)] as const,
            ([active, ids], prev) => {
                const doc = props.docDrag;
                if (!doc || !prev || !active || shouldReduceMotion("feedback")) return;
                const [wasActive] = prev;
                if (wasActive && wasActive !== active && !ids.includes(wasActive)) markLanded(active, doc.blockId);
            }
        )
    );
}

function StripTabs<T>(props: { strip: PaneTabStripProps<T> }): JSX.Element {
    const p = props.strip;
    return (
        <For each={p.tabs}>
            {(tab) => (
                <PaneTabStripItem
                    tab={tab}
                    active={p.activeId === p.getId(tab)}
                    getId={p.getId}
                    getLabel={p.getLabel}
                    getIcon={p.getIcon}
                    getTooltip={p.getTooltip}
                    getTooltipDetail={p.getTooltipDetail}
                    getTooltipSeat={p.getTooltipSeat}
                    getTooltipModel={p.getTooltipModel}
                    getAttention={p.getAttention}
                    getTabClass={p.getTabClass}
                    getColor={p.getColor}
                    flashOnActivity={p.flashOnActivity}
                    useProgress={p.useProgress}
                    collapsed={p.collapsed}
                    isClosing={p.isClosing}
                    onContextMenu={p.onTabContextMenu}
                    onActivate={p.onActivate}
                    onSelect={p.onSelect}
                    selection={p.selection}
                    onClose={p.onClose}
                    onDoubleClick={p.onTabDoubleClick}
                    renderLabel={p.renderLabel}
                    onReorder={p.onReorder}
                    paneKey={p.paneKey}
                    sourceTabId={p.sourceTabId}
                    docDrag={p.docDrag}
                />
            )}
        </For>
    );
}

function AddButton<T>(props: { strip: PaneTabStripProps<T> }): JSX.Element {
    const p = props.strip;
    return (
        <Tooltip divClassName="pane-tab-strip-add-tip" content={p.addTitle ?? "New tab"} delayMs={0}>
            <button
                type="button"
                class={`pane-tab-strip-add${p.addLabel ? " pane-tab-strip-add-labeled" : ""}`}
                aria-label={p.addLabel ?? p.addTitle ?? "New tab"}
                onClick={(e) => p.onAdd!(e)}
            >
                {/* Drawn, not a "+" character: a glyph is centred by its
                    font's line box, not its ink, so it sat off centre
                    (PaneTabStrip.scss's .pane-tab-strip-add-glyph). The
                    button's aria-label names it. */}
                <span class="pane-tab-strip-add-glyph" aria-hidden="true" />
                <Show when={p.addLabel}>
                    <span class="pane-tab-strip-add-label">{p.addLabel}</span>
                </Show>
            </button>
        </Tooltip>
    );
}

export function PaneTabStrip<T>(props: PaneTabStripProps<T>): JSX.Element {
    const s: StripState = createStripState(props);

    const { foreignHover, flash, incoming, highlightStrip } = useForeignTabDrop(props, () => s.refs.strip);

    useStripWidthAnimation({
        stripEl: () => s.refs.strip,
        tabCount: () => props.tabs.length,
        animateWidth: () => props.animateWidth,
        reveal: s.reveal,
    });

    trackLanding(props);

    useStripScrollInput(() => s.refs.strip, s.reveal);

    return (
        <div
            class="pane-tab-strip"
            classList={{
                "pane-tab-strip--foreign-hover": foreignHover() && highlightStrip(),
                "pane-tab-strip--no-blink": foreignHover() && highlightStrip() && flash() === "steady",
                "pane-tab-strip--no-invert": foreignHover() && highlightStrip() && flash() === "plain",
                "has-more-left": s.edges.moreLeft(),
                "has-more-right": s.edges.moreRight(),
                "pane-tab-strip--flush-left": s.edges.flushLeft(),
            }}
            ref={(el) => { s.refs.strip = el; }}
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
                <StripTabs strip={props} />
                <Show when={foreignHover() && incoming()}>
                    {/* Where the tab(s) being dragged here land: after the last tab, as the drop appends them. */}
                    <div class="pane-tab pane-tab--incoming" aria-hidden="true">
                        <span class="pane-tab-label">{incoming()}</span>
                    </div>
                </Show>
                {/* Tooltip (Portal-based), same reason as the per-tab one: native `title` is slow/inconsistent
                    in CEF. delayMs={0} — instant, matching the status bar's own tooltip feel (that one's a
                    separate data-tip/Portal system, but the same "no perceptible delay" intent). */}
                <Show when={props.onAdd}>
                    <AddButton strip={props} />
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
    getTooltipSeat?: (tab: T) => string | undefined;
    getTooltipModel?: (tab: T) => string | undefined;
    getAttention?: (tab: T) => boolean;
    getTabClass?: (tab: T) => Record<string, boolean>;
    getColor?: (tab: T) => PaneTabColors | undefined;
    flashOnActivity?: boolean;
    useProgress?: (tab: T) => Accessor<PaneTabProgress | null>;
    collapsed?: Accessor<boolean>;
    isClosing?: (tab: T) => boolean;
    onContextMenu?: (tab: T, e: MouseEvent) => void;
    onActivate: (id: string) => void;
    onSelect?: (id: string) => void;
    selection?: PaneTabSelectionApi;
    onClose?: (id: string) => void;
    onDoubleClick?: (tab: T) => void;
    renderLabel?: (tab: T) => JSX.Element;
    onReorder?: (blockId: string, targetId: string, position: "before" | "after") => boolean | void;
    paneKey?: string;
    sourceTabId?: string;
    docDrag?: DocTabDrag;
}

/**
 * The pill that just landed plays the landing bounce (`.pane-tab--landing` marks it while it
 * does, for the strip's reveal to wait out). On the pill, not its `.pane-tab-tip` wrapper: the
 * wrapper owns the flex sizing, so a transform here never moves its neighbours.
 */
function usePillLanding<T>(props: PaneTabStripItemProps<T>, id: () => string, pill: () => Element | undefined) {
    // The pane a landing bounce is for: the block, for document tabs.
    const landingKey = () => props.docDrag?.blockId ?? props.paneKey;
    const landed = () => !!landedTab()?.ids.has(id()) && landedTab()?.paneKey === landingKey();
    usePulseWhen(pill, landed);
    return { landingKey, landed };
}

// Applies to every pill, active included: `--pane-tab-bg` is what both
// the resting rule and `&--active` resolve, so a selected tab paints
// its own color too and only falls back to the plain
// `--block-bg-color` when it has none
// (SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md §2.2). Read via a CSS
// custom property rather than a direct inline `background-color` so the
// stylesheet keeps control of precedence across the resting / hover /
// active rules, same pattern as tab.tsx's `--tab-bg` / `--tab-bg-active` / `--tab-underline`.
function pillColorStyle(c: PaneTabColors | undefined): JSX.CSSProperties {
    if (!c) return {};
    return {
        ...(c.background ? { "--pane-tab-bg": c.background } : {}),
        ...(c.activeBackground ? { "--pane-tab-bg-active": c.activeBackground } : {}),
        ...(c.underline ? { "--pane-tab-underline": c.underline } : {}),
        ...(c.neutralBackground ? { "--pane-tab-neutral-bg": c.neutralBackground } : {}),
    } as JSX.CSSProperties;
}

/** What the pill's class list reads besides its props. */
interface PillClassState {
    id: () => string;
    attention: () => boolean;
    closing: () => boolean;
    dragging: () => boolean;
    dropSide: () => "before" | "after" | null;
    landed: () => boolean;
}

function pillClassList<T>(props: PaneTabStripItemProps<T>, s: PillClassState): Record<string, boolean> {
    return {
        "pane-tab--active": props.active,
        "pane-tab--attention": s.attention(),
        "pane-tab--closing": s.closing(),
        "pane-tab--dragging": s.dragging(),
        "pane-tab--drop-before": s.dropSide() === "before",
        "pane-tab--drop-after": s.dropSide() === "after",
        "pane-tab--landing": s.landed(),
        "pane-tab--selected": !!props.selection?.isSelected(s.id()),
        // Gates PaneTabStrip.scss's lighter-on-hover treatment —
        // only a tab with its own color gets it; every other
        // consumer's plain hover tint is untouched.
        "pane-tab--colored": !!props.getColor?.(props.tab)?.background,
        ...(props.getTabClass?.(props.tab) ?? {}),
    };
}

/** What the pill's content reads of the pill: its props, its state and its handlers. */
interface PillContentProps<T> {
    item: PaneTabStripItemProps<T>;
    id: () => string;
    attention: () => boolean;
    closing: () => boolean;
    progress: ReturnType<typeof usePillProgress>;
    windowDisplayed: Accessor<boolean>;
    onCloseClick: (e: MouseEvent) => void;
}

/** The progress line and its words, the selected mark, the icon, the label and the close button, in that order. */
function PillContent<T>(p: PillContentProps<T>): JSX.Element {
    const item = p.item;
    const { lineState, lineLook, progressInfo, descriptionId } = p.progress;
    return (
        <>
            <Show when={item.useProgress}>
                <span
                    class="pane-tab-progress"
                    classList={{
                        "pane-tab-progress--on": lineState() != null,
                        "pane-tab-progress--sweeping": lineState() === "working" && p.windowDisplayed(),
                    }}
                    data-state={lineLook() ?? undefined}
                    aria-hidden="true"
                >
                    <span class="pane-tab-progress-bar" />
                </span>
                <Show when={progressInfo()}>
                    {(info) => (
                        <span id={descriptionId()} class="pane-tab-state-text">
                            {info().text}
                        </span>
                    )}
                </Show>
            </Show>
            <Show when={item.selection?.isSelected(p.id())}>
                {/* The non-colour cue for "selected"; the label is what a screen reader reads. */}
                <span class="pane-tab-selected-mark" role="img" aria-label="Selected">
                    <i class="fa-solid fa-circle-check" aria-hidden="true" />
                </span>
            </Show>
            {item.getIcon && <span class="pane-tab-icon">{item.getIcon(item.tab)}</span>}
            {item.renderLabel ? (
                item.renderLabel(item.tab)
            ) : (
                <span class="pane-tab-label">{item.getLabel(item.tab)}</span>
            )}
            <Show when={item.onClose}>
                {/* Always in the DOM for attention tabs (closing those may need confirmation); hover-shown
                    otherwise, purely via CSS opacity. */}
                <CloseButton
                    size="tab"
                    class="pane-tab-close"
                    tooltip={false}
                    {...tip(p.closing() ? "Closing…" : p.attention() ? "Close (unsaved changes)" : "Close")}
                    label={p.closing() ? "Closing tab" : "Close tab"}
                    busy={p.closing()}
                    onClick={p.onCloseClick}
                />
            </Show>
        </>
    );
}

/** Whether the pill is closing, and its close button's click. */
function usePillClose<T>(props: PaneTabStripItemProps<T>, id: () => string) {
    const closing = () => props.isClosing?.(props.tab) ?? false;
    const onCloseClick = (e: MouseEvent) => {
        e.stopPropagation();
        if (closing()) return;
        props.onClose?.(id());
    };
    return { closing, onCloseClick };
}

function PaneTabStripItem<T>(props: PaneTabStripItemProps<T>): JSX.Element {
    const id = () => props.getId(props.tab);
    const attention = () => props.getAttention?.(props.tab) ?? false;
    let pillRef: HTMLDivElement | undefined;
    const { landingKey, landed } = usePillLanding(props, id, () => pillRef);
    const isDragging = () =>
        props.docDrag ? isDraggedDocTab(props.docDrag.blockId, id()) : isDraggedPaneTab(id(), props.paneKey);

    // Activity flash: click this pill on every tone from its own block. Not while its window
    // tab is hidden: nobody sees it, and each flash is a Web Animation of ~65 keyframes (the
    // window tab's own flash still shows the activity). The color comes from the
    // pill's own `--pane-tab-underline` in the stylesheet, so no base
    // color is passed here.
    const windowDisplayed = useWindowTabDisplayed();
    const progress = usePillProgress(props, id);
    const { progressInfo, descriptionId } = progress;
    usePillFlash({
        enabled: () => props.flashOnActivity,
        id,
        paneKey: () => props.paneKey,
        pill: () => pillRef,
        windowDisplayed,
    });
    const gestures = usePillGestures(props, id);
    const dropSide = usePillDrag({ props, id, landingKey, pill: () => pillRef, onDragStarted: gestures.noteDragStarted });
    const { onMouseDown, onClick, onDblClick } = gestures;

    const { closing, onCloseClick } = usePillClose(props, id);
    const colorStyle = () => pillColorStyle(props.getColor?.(props.tab));

    // Tooltip (Portal-based) rather than native `title` — the strip has
    // overflow:hidden, which would clip a CSS tooltip, and native `title`
    // is slow/inconsistent in CEF. The Tooltip's wrapper div carries the
    // flex sizing (.pane-tab-tip); the tab keeps its own mousedown/click/
    // dblclick handlers.
    return (
        <Tooltip
            placement="bottom"
            divClassName="pane-tab-tip"
            immediate={!!(props.getTooltipDetail?.(props.tab) || props.getTooltipSeat?.(props.tab) || props.getTooltipModel?.(props.tab) || progressInfo())}
            content={pillTooltipContent(props, progressInfo)}
        >
            <div
                class="pane-tab"
                classList={pillClassList(props, { id, attention, closing, dragging: isDragging, dropSide, landed })}
                style={colorStyle()}
                data-tab-id={id()}
                aria-busy={progressInfo()?.state === "working" ? "true" : undefined}
                aria-describedby={progressInfo() ? descriptionId() : undefined}
                ref={(el) => { pillRef = el; }}
                onMouseDown={onMouseDown}
                onClick={onClick}
                onDblClick={onDblClick}
                onContextMenu={(e) => props.onContextMenu?.(props.tab, e)}
            >
                <PillContent
                    item={props}
                    id={id}
                    attention={attention}
                    closing={closing}
                    progress={progress}
                    windowDisplayed={windowDisplayed}
                    onCloseClick={onCloseClick}
                />
            </div>
        </Tooltip>
    );
}
