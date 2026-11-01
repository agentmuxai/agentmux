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

import { createMemo, Match, Show, Switch, type JSX } from "solid-js";
import { blockViewToIcon, blockViewToName } from "@/app/block/blockutil";
import { getPaneTab } from "@/app/block/pane-tab-registry";
import { atoms } from "@/app/store/global";
import { makeIconClass } from "@/util/util";
import { FaviconImage } from "./FaviconImage";
import { ProviderLogo } from "./ProviderLogo";

export type PaneTabIcon =
    | { kind: "fa"; name: string; color?: string }
    | { kind: "provider"; provider: string }
    | { kind: "img"; src: string };

/** A small mark at an icon's bottom-right that says something about the tab (the browser identity it
 *  browses as), without replacing the icon. `color` is a CSS colour; `label` names what it marks, which the tab's tooltip says. */
export interface PaneTabBadge {
    color: string;
    label: string;
}

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
    /** A mark on the icon, which the icon (own or the live favicon) keeps showing under it. */
    badge?: (ctx: PaneTabContext) => PaneTabBadge | undefined;
    /** Returns a rename action when this tab can be renamed right now. */
    renamer?: (ctx: PaneTabContext) => ((title: string) => Promise<void>) | undefined;
}

export interface PaneTabInfo {
    blockId: string;
    label: string;
    icon: PaneTabIcon;
    badge?: PaneTabBadge;
    rename?: (title: string) => Promise<void>;
    /**
     * What the tab shows that came from its view's live name or favicon and is not yet in the block's
     * meta (`pane-tab-persist.ts` writes it). A tab opened after a reload has no live ViewModel until it
     * is clicked, so it reads the last real name from the meta instead of flipping to it on the click.
     */
    persist?: { name?: string; favicon?: string };
}

/** The last real live name and favicon of a tab, in the block's meta (written by `pane-tab-persist.ts`). */
export const TAB_NAME_META = "pane:label";
export const TAB_FAVICON_META = "pane:favicon";

// Descriptors are part of each view's manifest (`pane-tab-registry.ts`).
/** Live names/favicons exist only while a tab is active (dormant
 *  non-terminal tabs are unmounted), so each pane keeps the last value seen
 *  per tab — otherwise a tab would change identity the moment another tab is
 *  added. Owned by one pane's chrome and pruned to its current tabs. */
export interface PaneTabMemory {
    names: Map<string, string>;
    favicons: Map<string, string>;
    /** Tabs whose remembered name is a real one (seen live, or read from the meta), not a stand-in. */
    real: Set<string>;
}

export function createPaneTabMemory(): PaneTabMemory {
    return { names: new Map(), favicons: new Map(), real: new Set() };
}

export function prunePaneTabMemory(memory: PaneTabMemory, liveIds: Iterable<string>): void {
    const live = new Set(liveIds);
    for (const map of [memory.names, memory.favicons]) {
        for (const id of map.keys()) {
            if (!live.has(id)) map.delete(id);
        }
    }
    for (const id of memory.real) {
        if (!live.has(id)) memory.real.delete(id);
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
    const d = getPaneTab(ctx.view)?.tab;
    const widget = widgetForView(ctx.view);

    // A freshly built ViewModel can report a stand-in name before its real
    // one loads: a browser pane's ViewModel is rebuilt each time its tab
    // becomes active and says "Browser", then the URL's hostname, until the
    // page reports its title. Remembering those flashed the stand-ins over
    // the page title on every switch. The ViewModel says so explicitly
    // (`viewNameIsPlaceholder`) rather than being guessed from the string,
    // which would also freeze a tab whose real name happens to equal its
    // view's generic name. A stand-in only fills in when nothing better has
    // been seen for this tab yet.
    // A tab that has not been active since the pane mounted (after a reload, a move, a merge) has no
    // live ViewModel and no memory: it starts from the last real name and favicon its own meta holds, so
    // clicking it does not change them. The live value, once it is real, replaces them.
    const persistedName = ctx.meta?.[TAB_NAME_META];
    if (typeof persistedName === "string" && persistedName.length > 0 && !memory.names.has(ctx.blockId)) {
        memory.names.set(ctx.blockId, persistedName);
        memory.real.add(ctx.blockId);
    }
    const persistedFavicon = ctx.meta?.[TAB_FAVICON_META];
    if (typeof persistedFavicon === "string" && persistedFavicon.length > 0 && !memory.favicons.has(ctx.blockId)) {
        memory.favicons.set(ctx.blockId, persistedFavicon);
    }

    const liveName = readLive<string>(ctx.liveViewModel?.viewName);
    if (typeof liveName === "string" && liveName.length > 0) {
        const isPlaceholder = readLive<boolean>(ctx.liveViewModel?.viewNameIsPlaceholder) === true;
        if (!isPlaceholder || !memory.names.has(ctx.blockId)) memory.names.set(ctx.blockId, liveName);
        if (!isPlaceholder) memory.real.add(ctx.blockId);
    }
    const liveFavicon = readLive<string>(ctx.liveViewModel?.viewFaviconUrl);
    if (typeof liveFavicon === "string" && liveFavicon.length > 0) memory.favicons.set(ctx.blockId, liveFavicon);

    const ownLabel = (ctx.meta?.["frame:title"] as string | undefined) || labelOverride || d?.label?.(ctx);
    // What the view's registry entry names the tab from the meta alone. It comes before a remembered name, which
    // can be older than the meta (a section changed while the tab was behind another), and after a name the
    // mounted view reports itself, which refines it.
    const liveReal =
        typeof liveName === "string" && liveName.length > 0 && readLive<boolean>(ctx.liveViewModel?.viewNameIsPlaceholder) !== true;
    const metaTitle = ownLabel || liveReal ? undefined : getPaneTab(ctx.view)?.title?.(ctx.meta) || undefined;
    const label = ownLabel || metaTitle || memory.names.get(ctx.blockId) || widget?.label || blockViewToName(ctx.view);

    const frameIcon = ctx.meta?.["frame:icon"] as string | undefined;
    const ownIcon = (frameIcon ? faIcon(frameIcon, ctx.meta) : undefined) ?? d?.icon?.(ctx);
    const favicon = memory.favicons.get(ctx.blockId);
    const icon: PaneTabIcon =
        ownIcon ?? (favicon ? { kind: "img", src: favicon } : undefined) ?? faIcon(widget?.icon || blockViewToIcon(ctx.view), ctx.meta);

    // What the view named the tab, when that is what the tab shows, and the meta does not hold it yet.
    const name = !ownLabel && !metaTitle && memory.real.has(ctx.blockId) ? memory.names.get(ctx.blockId) : undefined;
    const persist = {
        name: name !== undefined && name !== persistedName ? name : undefined,
        favicon: !ownIcon && favicon !== undefined && favicon !== persistedFavicon ? favicon : undefined,
    };
    return {
        blockId: ctx.blockId,
        label,
        icon,
        badge: d?.badge?.(ctx),
        rename: d?.renamer?.(ctx),
        persist: persist.name !== undefined || persist.favicon !== undefined ? persist : undefined,
    };
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
export function PaneTabIconView(props: { icon: () => PaneTabIcon | undefined; badge?: () => PaneTabBadge | undefined }): JSX.Element {
    const icon = createMemo(props.icon, undefined, { equals: sameIcon });
    return (
        <>
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
                {(img) => <FaviconImage src={img().src} />}
            </Match>
        </Switch>
        <Show when={props.badge?.()}>
            {(badge) => <span class="pane-tab-icon-badge" style={{ background: badge().color }} aria-hidden="true" />}
        </Show>
        </>
    );
}
