// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The ONE registry of pane tab types — Pane Tab contract v1, Phase 2
 * (docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3/§4).
 *
 * Everything the host needs to know about a view type WITHOUT an instance
 * lives in its manifest: its name and aliases, label and icon, whether its
 * tabs stay mounted, and how it describes itself as a tab pill. The view →
 * ViewModel class map (`block-registry.ts`), the migration aliases
 * (`block.tsx`), the default label/icon tables (`blockutil.tsx`),
 * the keep-alive set (`pane-leaf-chrome.tsx`) and the parallel pane-tab
 * descriptor registry (`pane-tab-model.tsx`) are all derived from it.
 *
 * Pure on purpose: this module imports no view. Built-in views register in
 * `block-registry.ts`; a future user-installed widget registers the same way.
 */

import type { PaneTabDescriptor } from "@/app/element/pane-tab-model";
import type { NodeModel } from "@/layout/index";
import type { Accessor, JSX } from "solid-js";

/** What a view type declares instead of shared code checking its name
 *  (Pane Tab contract Phase 5, spec §2.4 #5). Every field is optional; the
 *  default is what an unlisted view type always got. */
export interface PaneTabCapabilities {
    /** Per TAB, never per pane: a `keepAlive` tab stays mounted while inactive,
     *  a `remount` tab (the default) is unmounted. */
    lifecycle?: "remount" | "keepAlive";
    /** Its content is a native surface composited above the DOM (the browser's
     *  page): it must collapse whenever the tab isn't visible. */
    nativeSurface?: boolean;
    /** `"surface"`: an uncolored header keeps the theme's block surface (the
     *  agent pane) instead of the one fixed default header color every other
     *  pane gets. */
    header?: "default" | "surface";
    /** The header shows a mic button (the view model's `voiceHandle`), with
     *  this tooltip. A view that takes voice elsewhere (agent: beside its
     *  composer) leaves this out. */
    headerMic?: { title: string };
    /** A boolean setting that hides this view's CPU/memory stats badge when
     *  set to `false` (terminal: `term:showstatsbadge`). Without it the badge
     *  always shows. */
    statsBadgeSetting?: string;
    /** `frame:hue` (or `frame:activebordercolor`) colors this tab's active
     *  border — a terminal running an agent CLI shows that agent's color. */
    hueBorder?: boolean;
    /** Takes part in per-pane zoom (`term:zoom`, Ctrl+Scroll, the all-panes
     *  batch), scaling from `baseFontSize` (default 15) unless the block sets
     *  `term:fontsize`. */
    paneZoom?: { baseFontSize?: number };
    /** Accepts typed/pasted text, so the pane menu offers Paste into it. */
    acceptsInput?: boolean;
    /** Ctrl+key combinations belong to the content (a shell), so app
     *  shortcuts that would shadow one (Ctrl+F search) stand down. */
    shellKeys?: boolean;
    /** A new block created while this one is focused starts in its
     *  `cmd:cwd`. */
    sharesCwd?: boolean;
    /** Meta keys a split of this pane does NOT copy into the new pane (agent:
     *  its agent-specific fields, so the new pane opens the picker). */
    splitDropsMeta?: string[];
    /** The pane runs against a connection (`meta.connection`) and its header
     *  shows the connection button (sysinfo, term). */
    connection?: boolean;
}

/** What the host gives a native instance — its only way in (no raw
 *  nodeModel, MOS or RpcApi). */
export interface PaneTabHostContext {
    blockId: string;
    /** The block's meta, reactive. */
    meta: Accessor<MetaType | undefined>;
    /** Merges `patch` into the block's meta (a `null` value removes the key). */
    setMeta(patch: Record<string, unknown>): Promise<void>;
    isFocused: Accessor<boolean>;
    /** The ONE active/dormant/hidden signal (Phase 3, `usePaneTabVisibility`):
     *  a `nativeSurface` view collapses its surface whenever it isn't
     *  `"active"`. */
    visibility: Accessor<"active" | "dormant" | "windowHidden">;
}

/** A live native tab. The host decides the header, chrome and hiding. */
export interface PaneTabInstance {
    component: (props: { ctx: PaneTabHostContext }) => JSX.Element;
    /** Live title; `placeholder: true` while it is only a stand-in. */
    liveTitle?: Accessor<{ text: string; placeholder?: boolean }>;
    liveFavicon?: Accessor<string>;
    headerText?: Accessor<string | HeaderElem[]>;
    headerActions?: Accessor<(IconButtonDecl | ToggleIconButtonDecl)[]>;
    contextMenu?(ctx?: unknown): ContextMenuItem[];
    /** Items for the header's settings menu. */
    settingsMenu?(): ContextMenuItem[];
    focus?(): boolean;
    onKeyDown?(e: MuxKeyboardEvent): boolean;
    /** Fired by the host on BOTH paths when the tab becomes visible, and when
     *  it stops being visible (Phase 3b). A tab that becomes visible in a
     *  focused pane also gets `focus()`. */
    onActivate?(): void;
    onDeactivate?(): void;
    /** Runs when the host disposes the instance; its reactive root (host rule
     *  8) is disposed right after. */
    dispose?(): void;
}

export interface PaneTabManifest {
    apiVersion: 1;
    /** The block's `meta.view` value. */
    view: string;
    /** Old `meta.view` values still found in persisted blocks, redirected here. */
    aliases?: string[];
    label: string;
    /** Font Awesome icon name. */
    icon: string;
    capabilities?: PaneTabCapabilities;
    /** How a block of this view becomes a tab pill, beyond label and icon. */
    tab?: PaneTabDescriptor;
    /** What this view type contributes to the shared pane chrome while one
     *  of its tabs is the active one (Pane Tab contract Phase 4). Built once
     *  per pane, in the chrome's own reactive scope, the first time a tab of
     *  this view type is active there; `anchorBlockId` is that tab. */
    chrome?: (anchorBlockId: string, nodeModel: NodeModel) => PaneChromeModel;
    /** Native instance factory (Phase 2b). Called by the host in the
     *  instance's own reactive root. Exactly one of `create` and
     *  `viewModelClass`. */
    create?(ctx: PaneTabHostContext): PaneTabInstance;
    /** Legacy instance factory: an existing ViewModel class (`legacyAdapter`).
     *  Replaced by `create` as views migrate. */
    viewModelClass?: ViewModelClass;
}

const manifests = new Map<string, PaneTabManifest>();
const aliasToView = new Map<string, string>();

/** Registers a pane tab type. Returns its unregister function. Throws when the
 *  view or one of its aliases is already taken — a silent overwrite would let
 *  two widgets fight over the same blocks. */
export function registerPaneTab(manifest: PaneTabManifest): () => void {
    if ((manifest.create == null) === (manifest.viewModelClass == null)) {
        throw new Error(`pane tab "${manifest.view}" needs exactly one of create and viewModelClass`);
    }
    const names = [manifest.view, ...(manifest.aliases ?? [])];
    for (const name of names) {
        if (manifests.has(name) || aliasToView.has(name)) {
            throw new Error(`pane tab "${name}" is already registered`);
        }
    }
    manifests.set(manifest.view, manifest);
    for (const alias of manifest.aliases ?? []) aliasToView.set(alias, manifest.view);
    return () => {
        if (manifests.get(manifest.view) !== manifest) return;
        manifests.delete(manifest.view);
        for (const alias of manifest.aliases ?? []) {
            if (aliasToView.get(alias) === manifest.view) aliasToView.delete(alias);
        }
    };
}

/** A manifest for an existing ViewModel class, so views register unchanged. */
export function legacyAdapter(
    view: string,
    viewModelClass: ViewModelClass,
    opts: {
        label?: string;
        icon?: string;
        aliases?: string[];
        lifecycle?: PaneTabCapabilities["lifecycle"];
        capabilities?: Omit<PaneTabCapabilities, "lifecycle">;
        tab?: PaneTabDescriptor;
        chrome?: PaneTabManifest["chrome"];
    } = {}
): PaneTabManifest {
    return {
        apiVersion: 1,
        view,
        aliases: opts.aliases,
        label: opts.label ?? view,
        icon: opts.icon ?? "square",
        capabilities:
            opts.lifecycle || opts.capabilities
                ? { ...opts.capabilities, ...(opts.lifecycle ? { lifecycle: opts.lifecycle } : {}) }
                : undefined,
        tab: opts.tab,
        chrome: opts.chrome,
        viewModelClass,
    };
}

/** The canonical view for a `meta.view` value: an alias maps to its view,
 *  anything else is returned unchanged. */
export function resolvePaneTabView(view: string): string {
    return aliasToView.get(view) ?? view;
}

export function getPaneTab(view: string | null | undefined): PaneTabManifest | undefined {
    if (!view) return undefined;
    return manifests.get(resolvePaneTabView(view));
}

export function isKeepAliveView(view: string | null | undefined): boolean {
    return getPaneTab(view)?.capabilities?.lifecycle === "keepAlive";
}

/** One declared capability of a view type (`undefined` when it doesn't
 *  declare it, or isn't registered). */
export function paneTabCapability<K extends keyof PaneTabCapabilities>(
    view: string | null | undefined,
    key: K
): PaneTabCapabilities[K] | undefined {
    return getPaneTab(view)?.capabilities?.[key];
}

export function paneTabLabelFor(view: string | null | undefined): string {
    if (!view) return "(No View)";
    return getPaneTab(view)?.label ?? view;
}

export function paneTabIconFor(view: string | null | undefined): string {
    return getPaneTab(view)?.icon ?? "square";
}
