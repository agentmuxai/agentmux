// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, createUniqueId, For, type JSX, onCleanup, onMount, Show } from "solid-js";
import { Tooltip } from "../tooltip";
import { densityClass, rovingTarget, UiIcon, type UiDensity } from "./shared";

import "./ui.scss";

export interface TabItem<T extends string = string> {
    id: T;
    label: string;
    icon?: string;
    /** Tooltip text when only the icon shows. Defaults to the label. */
    tooltip?: string;
    /** Extra class on the tab, e.g. a standing highlight on one section. */
    class?: string;
}

export function tabId(idPrefix: string, id: string): string {
    return `${idPrefix}-tab-${id}`;
}

export function tabPanelId(idPrefix: string): string {
    return `${idPrefix}-panel`;
}

export interface TabsProps<T extends string> {
    items: TabItem<T>[];
    value: T;
    onChange: (id: T) => void;
    /** Default `horizontal` (the Stash look); `vertical` is a rail. */
    orientation?: "horizontal" | "vertical";
    /** Hide labels; each tab keeps its label as its name and gets a tooltip. */
    iconOnly?: boolean;
    /**
     * When tabs show their tooltip: `icon-only` (default) only while `iconOnly`
     * hides the labels; `always` for a consumer that hides labels itself, e.g.
     * with a container query, as the Stash does.
     */
    tooltips?: "icon-only" | "always";
    density?: UiDensity;
    /** Prefix for the tab and panel ids that tie tabs to their panel. */
    idPrefix: string;
    ariaLabel: string;
    class?: string;
}

/**
 * Section navigation (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4).
 * The selected tab gets a 2px accent line, never a fill: underneath when
 * horizontal, on the leading edge when vertical. Arrow keys, Home and End
 * move the selection; only the selected tab is in the Tab order.
 */
export function Tabs<T extends string>(props: TabsProps<T>): JSX.Element {
    const buttons: HTMLButtonElement[] = [];
    const orientation = () => props.orientation ?? "horizontal";

    const onKeyDown = (e: KeyboardEvent, index: number) => {
        const target = rovingTarget(e.key, index, props.items.length, orientation());
        if (target == null) return;
        e.preventDefault();
        props.onChange(props.items[target].id);
        buttons[target]?.focus();
    };

    return (
        <div
            role="tablist"
            aria-label={props.ariaLabel}
            aria-orientation={orientation()}
            class={clsx(
                "ui-tabs",
                `ui-tabs--${orientation()}`,
                props.iconOnly && "ui-tabs--icon-only",
                densityClass(props.density),
                props.class
            )}
        >
            <For each={props.items}>
                {(item, i) => {
                    const selected = () => item.id === props.value;
                    return (
                        <Tooltip
                            content={item.tooltip ?? item.label}
                            placement={orientation() === "vertical" ? "right" : "bottom"}
                            disable={!props.iconOnly && props.tooltips !== "always"}
                            divClassName="ui-tooltip-anchor"
                        >
                            <button
                                ref={(el) => (buttons[i()] = el)}
                                type="button"
                                role="tab"
                                id={tabId(props.idPrefix, item.id)}
                                class={clsx("ui-tab", item.class)}
                                aria-selected={selected() ? "true" : "false"}
                                aria-controls={tabPanelId(props.idPrefix)}
                                aria-label={item.label}
                                tabIndex={selected() ? 0 : -1}
                                onClick={() => props.onChange(item.id)}
                                onKeyDown={(e) => onKeyDown(e, i())}
                            >
                                <Show when={item.icon}>{(icon) => <UiIcon name={icon()} />}</Show>
                                <span class="ui-tab-label">{item.label}</span>
                            </button>
                        </Tooltip>
                    );
                }}
            </For>
        </div>
    );
}

export type TabbedPaneLayout = "rail" | "rail-icons" | "top";

/** Which layout a TabbedPane of this width uses. */
export function tabbedPaneLayout(width: number, collapseBelow: number, topBelow: number): TabbedPaneLayout {
    if (width < topBelow) return "top";
    if (width < collapseBelow) return "rail-icons";
    return "rail";
}

export interface TabbedPaneProps<T extends string> {
    items: TabItem<T>[];
    value: T;
    onChange: (id: T) => void;
    /** Prefix for the tab and panel ids. Generated when omitted, so two open copies of a pane can't share ids. */
    idPrefix?: string;
    ariaLabel: string;
    density?: UiDensity;
    /** Width below which the rail shows icons only. Default 768. */
    collapseBelow?: number;
    /** Width below which the rail becomes tabs along the top. Default 480. */
    topBelow?: number;
    class?: string;
    /** Extra class on the panel that holds the content. */
    panelClass?: string;
    /** The selected section's content. */
    children: JSX.Element;
}

/**
 * A pane with section navigation and one panel. Wide: a rail with labels.
 * Narrower: an icon-only rail with tooltips. Narrowest: icon-only tabs
 * spread along the top, Stash-style. One tablist throughout; only its
 * layout changes.
 */
export function TabbedPane<T extends string>(props: TabbedPaneProps<T>): JSX.Element {
    let root: HTMLDivElement | undefined;
    const generatedPrefix = `ui-tabs-${createUniqueId()}`;
    const idPrefix = () => props.idPrefix ?? generatedPrefix;
    const [layout, setLayout] = createSignal<TabbedPaneLayout>("rail");

    onMount(() => {
        // jsdom (unit tests) has no ResizeObserver; the rail is the default.
        if (!root || typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver((entries) => {
            const width = entries[entries.length - 1].contentRect.width;
            setLayout(tabbedPaneLayout(width, props.collapseBelow ?? 768, props.topBelow ?? 480));
        });
        observer.observe(root);
        onCleanup(() => observer.disconnect());
    });

    return (
        <div ref={root} class={clsx("ui-tabbed-pane", `ui-tabbed-pane--${layout()}`, densityClass(props.density), props.class)}>
            <Tabs
                items={props.items}
                value={props.value}
                onChange={props.onChange}
                orientation={layout() === "top" ? "horizontal" : "vertical"}
                iconOnly={layout() !== "rail"}
                idPrefix={idPrefix()}
                ariaLabel={props.ariaLabel}
            />
            <div
                class={clsx("ui-tabbed-pane-panel", props.panelClass)}
                role="tabpanel"
                id={tabPanelId(idPrefix())}
                aria-labelledby={tabId(idPrefix(), props.value)}
            >
                {props.children}
            </div>
        </div>
    );
}
