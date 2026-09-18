// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The ONE pane-tab model: how a block becomes a tab pill (label + icon +
 * optional rename), shared by every pane type. `PaneChrome` derives every
 * tab through `describePaneTab`; a view type that needs more than the
 * generic rules (agent's provider logo / agentName, terminal's
 * "Terminal N") registers a `PaneTabDescriptor` instead of building its own
 * parallel tab list.
 *
 * Icons are data (`PaneTabIcon`), not pre-built elements, and always render
 * through `PaneTabIconView` at one fixed size — so a tab's icon can't
 * vanish or change size as tabs are added, switched, or closed.
 */

import { createMemo, Match, Switch, type JSX } from "solid-js";
import { blockViewToIcon, blockViewToName } from "@/app/block/blockutil";
import { atoms } from "@/app/store/global";
import { makeIconClass } from "@/util/util";
import { ProviderLogo } from "./ProviderLogo";

export type PaneTabIcon =
    | { kind: "fa"; name: string; color?: string }
    | { kind: "provider"; provider: string }
    | { kind: "img"; src: string };

export interface PaneTabContext {
    blockId: string;
    view: string | undefined;
    meta: MetaType | undefined;
    /** 1-based position among this pane's stack members of the same view
     *  type (0 for a tab that lives in another pane). */
    ordinal: number;
    /** The mounted ViewModel — only ever set for the active tab. */
    liveViewModel: ViewModel | null;
}

export interface PaneTabDescriptor {
    label?: (ctx: PaneTabContext) => string | undefined;
    icon?: (ctx: PaneTabContext) => PaneTabIcon | undefined;
    /** Returns a rename action when this tab can be renamed right now. */
    renamer?: (ctx: PaneTabContext) => ((title: string) => Promise<void>) | undefined;
}

export interface PaneTabInfo {
    blockId: string;
    label: string;
    icon: PaneTabIcon;
    rename?: (title: string) => Promise<void>;
}

const descriptors = new Map<string, PaneTabDescriptor>();

export function registerPaneTabDescriptor(view: string, descriptor: PaneTabDescriptor): void {
    descriptors.set(view, descriptor);
}

/** Live names/favicons exist only while a tab is active (dormant
 *  non-terminal tabs are unmounted), so each pane keeps the last value seen
 *  per tab — otherwise a tab would change identity the moment another tab is
 *  added. Owned by one pane's chrome and pruned to its current tabs. */
export interface PaneTabMemory {
    names: Map<string, string>;
    favicons: Map<string, string>;
}

export function createPaneTabMemory(): PaneTabMemory {
    return { names: new Map(), favicons: new Map() };
}

export function prunePaneTabMemory(memory: PaneTabMemory, liveIds: Iterable<string>): void {
    const live = new Set(liveIds);
    for (const map of [memory.names, memory.favicons]) {
        for (const id of map.keys()) {
            if (!live.has(id)) map.delete(id);
        }
    }
}

const ICON_COLOR_RE = /^((#[0-9a-f]{6,8})|([a-z]+))$/;

function faIcon(name: string, meta: MetaType | undefined): PaneTabIcon {
    const color = meta?.["icon:color"];
    return typeof color === "string" && ICON_COLOR_RE.test(color) ? { kind: "fa", name, color } : { kind: "fa", name };
}

/** The widget-bar entry (widgets.json) that opens this view, so a tab and
 *  the button that opened it share a name and icon. Prefers
 *  `defwidget@<view>` — several widgets (Slack, Discord, …) are really
 *  `browser` panes. */
function widgetForView(view: string | undefined): WidgetConfigType | undefined {
    if (!view) return undefined;
    const widgets = atoms.fullConfigAtom()?.widgets ?? {};
    const own = widgets[`defwidget@${view}`];
    if (own?.blockdef?.meta?.view === view) return own;
    return Object.values(widgets).find((w) => w?.blockdef?.meta?.view === view);
}

function readLive<T>(accessor: unknown): T | undefined {
    return typeof accessor === "function" ? (accessor as () => T)() : undefined;
}

export function describePaneTab(
    ctx: PaneTabContext,
    labelOverride?: string,
    memory: PaneTabMemory = createPaneTabMemory()
): PaneTabInfo {
    const d = ctx.view ? descriptors.get(ctx.view) : undefined;

    const liveName = readLive<string>(ctx.liveViewModel?.viewName);
    if (typeof liveName === "string" && liveName.length > 0) memory.names.set(ctx.blockId, liveName);
    const liveFavicon = readLive<string>(ctx.liveViewModel?.viewFaviconUrl);
    if (typeof liveFavicon === "string" && liveFavicon.length > 0) memory.favicons.set(ctx.blockId, liveFavicon);

    const widget = widgetForView(ctx.view);
    const label =
        (ctx.meta?.["frame:title"] as string | undefined) ||
        labelOverride ||
        d?.label?.(ctx) ||
        memory.names.get(ctx.blockId) ||
        widget?.label ||
        blockViewToName(ctx.view);

    const frameIcon = ctx.meta?.["frame:icon"] as string | undefined;
    const favicon = memory.favicons.get(ctx.blockId);
    const icon: PaneTabIcon =
        (frameIcon ? faIcon(frameIcon, ctx.meta) : undefined) ??
        d?.icon?.(ctx) ??
        (favicon ? { kind: "img", src: favicon } : undefined) ??
        faIcon(widget?.icon || blockViewToIcon(ctx.view), ctx.meta);

    return { blockId: ctx.blockId, label, icon, rename: d?.renamer?.(ctx) };
}

function sameIcon(a: PaneTabIcon | undefined, b: PaneTabIcon | undefined): boolean {
    if (a === b) return true;
    if (!a || !b || a.kind !== b.kind) return false;
    switch (a.kind) {
        case "fa":
            return a.name === (b as typeof a).name && a.color === (b as typeof a).color;
        case "provider":
            return a.provider === (b as typeof a).provider;
        case "img":
            return a.src === (b as typeof a).src;
    }
}

/** Renders a tab icon (PaneTabStrip supplies the fixed-size `.pane-tab-icon`
 *  box). `icon` is an accessor so the element is only rebuilt when the icon
 *  actually changes, not on every tab-list recompute. */
export function PaneTabIconView(props: { icon: () => PaneTabIcon | undefined }): JSX.Element {
    const icon = createMemo(props.icon, undefined, { equals: sameIcon });
    return (
        <Switch>
            <Match when={icon()?.kind === "fa" && (icon() as Extract<PaneTabIcon, { kind: "fa" }>)}>
                {(fa) => (
                    <i
                        class={makeIconClass(fa().name, true, { defaultIcon: "square" })}
                        style={fa().color ? { color: fa().color } : undefined}
                    />
                )}
            </Match>
            <Match when={icon()?.kind === "provider" && (icon() as Extract<PaneTabIcon, { kind: "provider" }>)}>
                {(p) => <ProviderLogo provider={p().provider} size={14} />}
            </Match>
            <Match when={icon()?.kind === "img" && (icon() as Extract<PaneTabIcon, { kind: "img" }>)}>
                {(img) => <img src={img().src} alt="" aria-hidden="true" />}
            </Match>
        </Switch>
    );
}
