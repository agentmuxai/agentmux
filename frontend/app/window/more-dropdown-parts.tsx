// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The More dropdown's rows: a plain widget row, and a parent row that expands a flyout of its children. */

import { makeIconClass } from "@/util/util";
import { Show, type JSX } from "solid-js";
import { widgetIconStyle } from "./action-widgets-config";
import { PinnedWidgetFlyout } from "./pinned-widget-flyout";
import { parentFlyoutHover, parentRowHover, type MoreDropdownState } from "./more-dropdown-state";

/** A row that opens its widget on click. */
export function PlainRow(props: { s: MoreDropdownState; itemKey: string; widget: WidgetConfigType }): JSX.Element {
    const s = props.s;
    const key = props.itemKey;
    const widget = props.widget;
    return (
        <div
            class="action-widget-more-item"
            onMouseEnter={(e) => s.parentPeers.closeOthers(key, e)}
            onClick={(e) => s.handleItemClick(widget, e)}
            onContextMenu={(e) => s.handleItemContextMenu(e, key)}
        >
            <span class="action-widget-more-item-icon widget-icon" style={widgetIconStyle(widget)}>
                <i class={makeIconClass(widget.icon, true, { defaultIcon: "window-maximize" })}></i>
            </span>
            <span class="action-widget-more-item-label">{widget.label}</span>
        </div>
    );
}

/** A row with children: hovering it opens a flyout beside it, which counts as part of the dropdown. */
export function ParentRow(props: {
    s: MoreDropdownState;
    itemKey: string;
    widget: WidgetConfigType;
    wmap: () => Record<string, WidgetConfigType>;
    settings: () => Record<string, any>;
    onClose: () => void;
    onItemContextMenu: (pos: { x: number; y: number }, key: string) => void;
    onSubmenuEnter?: () => void;
    onSubmenuLeave?: (e: MouseEvent) => void;
    isItemMenuOpen?: () => boolean;
}): JSX.Element {
    const s = props.s;
    const key = props.itemKey;
    const widget = props.widget;
    let rowEl: HTMLDivElement | undefined;
    const hover = s.parentHoverFor(key);

    const rowHover = parentRowHover(key, s.parentPeers, hover, props.isItemMenuOpen);
    const flyoutHover = parentFlyoutHover(key, hover, s.region, {
        onSubmenuEnter: props.onSubmenuEnter,
        onSubmenuLeave: props.onSubmenuLeave,
        isItemMenuOpen: props.isItemMenuOpen,
    });

    return (
        <>
            <div
                ref={(el) => (rowEl = el)}
                class="action-widget-more-item"
                onMouseEnter={rowHover.enter}
                onMouseLeave={rowHover.leave}
                onContextMenu={(e) => s.handleItemContextMenu(e, key)}
            >
                <span class="action-widget-more-item-icon widget-icon" style={widgetIconStyle(widget)}>
                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "window-maximize" })}></i>
                </span>
                <span class="action-widget-more-item-label">{widget.label}</span>
                <i class="fa-solid fa-chevron-right action-widget-more-item-chevron" />
            </div>
            <Show when={s.visibleParentSubMenus()[key]}>
                <PinnedWidgetFlyout
                    widget={widget}
                    wmap={props.wmap}
                    settings={props.settings}
                    onClose={() => {
                        s.closeParentSub(key);
                        hover.close();
                        props.onClose();
                    }}
                    onItemContextMenu={props.onItemContextMenu}
                    anchor={() => rowEl ?? null}
                    placement="right-start"
                    gutter={8}
                    {...flyoutHover}
                />
            </Show>
        </>
    );
}
