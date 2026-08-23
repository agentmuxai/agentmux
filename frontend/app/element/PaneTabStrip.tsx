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

import { createEffect, For, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { atoms } from "@/store/global";
import { Tooltip } from "./tooltip";
import "./PaneTabStrip.scss";

// Matches the other reveal-gate/cross-fade durations added alongside this
// one in SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.4.
const WIDTH_TRANSITION_MS = 160;

export interface PaneTabStripProps<T> {
    tabs: T[];
    activeId: string | null;

    /** This pane's own content zoom (term:zoom block meta) — agent's
     *  zoomFactor memo, editor's model.zoomAtom, terminal's
     *  model.termZoomAtom. NOT the global chrome-zoom control
     *  (window-header/status-bar's --zoomfactor) — deliberately per-pane,
     *  so tabs scale with the content they belong to, not uniformly
     *  across every pane in the window. Omit for 1 (unzoomed). See
     *  docs/specs/SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md §A. */
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
    /** Full tooltip text; falls back to the label when omitted. */
    getTooltip?: (tab: T) => string;
    /** "Attention" tabs (unsaved changes, needs-review, …) always show
     *  their close × instead of only on hover. */
    getAttention?: (tab: T) => boolean;
    /** Extra classes beyond active/attention (e.g. an editor preview tab's
     *  italic label, a fork's running/idle status accent). */
    getTabClass?: (tab: T) => Record<string, boolean>;

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
    onAdd?: () => void;
    addTitle?: string;
}

export function PaneTabStrip<T>(props: PaneTabStripProps<T>): JSX.Element {
    let stripRef: HTMLDivElement | undefined;
    let lastMeasuredWidth: number | undefined;
    let widthResetTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(widthResetTimeout));

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
    createEffect(
        on(
            () => props.tabs.length,
            () => {
                if (!props.animateWidth) return;
                const el = stripRef;
                if (!el) return;
                const newWidth = el.getBoundingClientRect().width;
                if (
                    lastMeasuredWidth !== undefined &&
                    lastMeasuredWidth !== newWidth &&
                    !atoms.prefersReducedMotionAtom()
                ) {
                    clearTimeout(widthResetTimeout);
                    el.style.transition = "none";
                    el.style.width = `${lastMeasuredWidth}px`;
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

    return (
        <div
            class="pane-tab-strip"
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
                            getTooltip={props.getTooltip}
                            getAttention={props.getAttention}
                            getTabClass={props.getTabClass}
                            onActivate={props.onActivate}
                            onClose={props.onClose}
                            onDoubleClick={props.onTabDoubleClick}
                            renderLabel={props.renderLabel}
                        />
                    )}
                </For>
                <Show when={props.onAdd}>
                    <button
                        type="button"
                        class="pane-tab-strip-add"
                        title={props.addTitle ?? "New tab"}
                        aria-label={props.addTitle ?? "New tab"}
                        onClick={() => props.onAdd!()}
                    >
                        +
                    </button>
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
    getTooltip?: (tab: T) => string;
    getAttention?: (tab: T) => boolean;
    getTabClass?: (tab: T) => Record<string, boolean>;
    onActivate: (id: string) => void;
    onClose?: (id: string) => void;
    onDoubleClick?: (tab: T) => void;
    renderLabel?: (tab: T) => JSX.Element;
}

function PaneTabStripItem<T>(props: PaneTabStripItemProps<T>): JSX.Element {
    const id = () => props.getId(props.tab);
    const attention = () => props.getAttention?.(props.tab) ?? false;

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
                    ...(props.getTabClass?.(props.tab) ?? {}),
                }}
                onMouseDown={onMouseDown}
                onClick={onClick}
                onDblClick={onDblClick}
            >
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
