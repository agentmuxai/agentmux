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
import type { Accessor, JSX } from "solid-js";

export interface PaneTabCapabilities {
    /** Per TAB, never per pane: a `keepAlive` tab stays mounted while inactive,
     *  a `remount` tab (the default) is unmounted. */
    lifecycle?: "remount" | "keepAlive";
}

/** What the host gives a native instance — its only way in (no raw
 *  nodeModel, MOS or RpcApi). `visibility` arrives with Phase 3. */
export interface PaneTabHostContext {
    blockId: string;
    /** The block's meta, reactive. */
    meta: Accessor<MetaType | undefined>;
    /** Merges `patch` into the block's meta (a `null` value removes the key). */
    setMeta(patch: Record<string, unknown>): Promise<void>;
    isFocused: Accessor<boolean>;
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
    focus?(): boolean;
    onKeyDown?(e: MuxKeyboardEvent): boolean;
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
        tab?: PaneTabDescriptor;
    } = {}
): PaneTabManifest {
    return {
        apiVersion: 1,
        view,
        aliases: opts.aliases,
        label: opts.label ?? view,
        icon: opts.icon ?? "square",
        capabilities: opts.lifecycle ? { lifecycle: opts.lifecycle } : undefined,
        tab: opts.tab,
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

export function paneTabLabelFor(view: string | null | undefined): string {
    if (!view) return "(No View)";
    return getPaneTab(view)?.label ?? view;
}

export function paneTabIconFor(view: string | null | undefined): string {
    return getPaneTab(view)?.icon ?? "square";
}
