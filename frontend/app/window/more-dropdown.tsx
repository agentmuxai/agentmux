// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MoreDropdown — floating overflow menu for the widget bar. Anchored to the
 * "More" button; lists widgets that aren't pinned (plus any pinned widgets
 * clipped by the responsive collapse). Extracted from action-widgets.tsx.
 */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { For, type JSX } from "solid-js";
import { createMoreDropdown, type Region } from "./more-dropdown-state";
import { ParentRow, PlainRow } from "./more-dropdown-parts";

const MoreDropdown = ({
    widgets,
    onClose,
    onItemContextMenu,
    anchor,
    settings,
    wmap,
    ref,
    onSubmenuEnter,
    onSubmenuLeave,
    setSubmenuEl,
    isItemMenuOpen,
}: {
    widgets: () => { key: string; widget: WidgetConfigType }[];
    onClose: () => void;
    onItemContextMenu: (pos: { x: number; y: number }, key: string) => void;
    anchor: () => HTMLElement | null;
    settings: () => Record<string, any>;
    wmap: () => Record<string, WidgetConfigType>;
    ref?: (el: HTMLDivElement) => void;
    /**
     * Optional hover-controller plumbing — present when the caller opens
     * this dropdown via hover-intent (a createSubmenuHover on the "More"
     * button itself) rather than click-only, so moving the cursor into this
     * panel cancels the pending close the same way any other hover-opened
     * flyout/submenu in the app does.
     */
    onSubmenuEnter?: () => void;
    onSubmenuLeave?: (e: MouseEvent) => void;
    setSubmenuEl?: (el: Region | null) => void;
    /**
     * True while the right-click item popover (pin/unpin, open-in-new-
     * window, ...) is open. A right-click's popover renders at the cursor
     * position, which makes the browser re-target hit-testing and fire a
     * spurious mouseleave on whatever's underneath even though the cursor
     * never actually moved — without this guard that leave reaches the
     * per-row hover controller for a Case B parent row (e.g. Messengers
     * nested inside More) and starts a close-intent, collapsing it out from
     * under the user mid right-click.
     */
    isItemMenuOpen?: () => boolean;
}): JSX.Element => {
    const s = createMoreDropdown({ onClose, onItemContextMenu, ref, setSubmenuEl });

    return (
        <AnchoredPopover
            anchor={anchor()}
            placement="bottom-end"
            gutter={4}
            hideUntilPlaced
            onDismiss={(reason) => reason === "escape" && onClose()}
            shellClass="anchored-popover--menu"
            class="action-widget-more-dropdown"
            ref={(el) => {
                s.registerBody(el);
                el.addEventListener("mouseenter", () => onSubmenuEnter?.());
                el.addEventListener("mouseleave", (e) => {
                    if (isItemMenuOpen?.()) return;
                    onSubmenuLeave?.(e);
                });
            }}
        >
            <For each={widgets()}>
                {({ key, widget }) => {
                    const isParent = (widget.children?.length ?? 0) > 0;
                    if (!isParent) return <PlainRow s={s} itemKey={key} widget={widget} />;

                    return (
                        <ParentRow
                            s={s}
                            itemKey={key}
                            widget={widget}
                            wmap={wmap}
                            settings={settings}
                            onClose={onClose}
                            onItemContextMenu={onItemContextMenu}
                            onSubmenuEnter={onSubmenuEnter}
                            onSubmenuLeave={onSubmenuLeave}
                            isItemMenuOpen={isItemMenuOpen}
                        />
                    );
                }}
            </For>
        </AnchoredPopover>
    );
};

MoreDropdown.displayName = "MoreDropdown";

export { MoreDropdown };
