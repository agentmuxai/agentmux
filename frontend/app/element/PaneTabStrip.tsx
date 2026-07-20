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

import { For, Show, type JSX } from "solid-js";
import { Tooltip } from "./tooltip";
import "./PaneTabStrip.scss";

export interface PaneTabStripProps<T> {
    tabs: T[];
    activeId: string | null;

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
    return (
        <div
            class="pane-tab-strip"
            // Double-click inside the strip should never bubble up and
            // maximize the pane — matches the icon-toggle pattern from
            // blockframe.tsx. True for every consumer, not just the editor.
            onDblClick={(e) => e.stopPropagation()}
        >
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
