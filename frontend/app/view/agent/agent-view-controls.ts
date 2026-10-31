// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's view (what it shows: virtualization/verbosity.ts) as the pane and everything
 * that drives it see it: the View popover, the composer chip, the pane menu, the command palette
 * and Alt+V. A pane follows the `agent:view` setting or keeps its own complete view in its meta
 * (docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.5).
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { getBlockMetaKeyAtom, getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { setBlockMeta } from "@/app/store/block-meta";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DocumentNode, DocumentState } from "./types";
import {
    nextPreset,
    resolveView,
    ruleOf,
    SWITCH_KEYS,
    VIEW_PRESETS,
    type AgentView,
    type PresetName,
    type ResolvedView,
    type SwitchKey,
} from "./virtualization/verbosity";

/** The setting and the pane meta key that hold a view. */
export const AGENT_VIEW_KEY = "agent:view";

export interface AgentViewControls {
    /** The view in force, and whether the pane has its own or follows the default. */
    resolved: Accessor<ResolvedView>;
    /** The default (`agent:view`), for the "Make this the default" button's state. */
    defaultView: Accessor<AgentView>;
    setPreset: (name: PresetName) => void;
    /** Change one switch; in a pane that follows the default this copies the effective view into the pane first. */
    setSwitch: <K extends SwitchKey>(key: K, value: AgentView[K]) => void;
    followDefault: () => void;
    makeDefault: () => void;
    /** Alt+V. */
    cycle: () => void;
    popoverOpen: Accessor<boolean>;
    setPopoverOpen: (open: boolean) => void;
}

/**
 * The rows whose fold the user made by hand (in `collapsedNodes`) and whose switch just changed:
 * changing a switch must do what it says, so those folds go; pins are kept.
 */
export function foldsToClear(nodes: readonly DocumentNode[], before: AgentView, after: AgentView): Set<string> {
    const changed = new Set(SWITCH_KEYS.filter((k) => before[k] !== after[k]));
    const ids = new Set<string>();
    if (changed.size === 0) return ids;
    for (const node of nodes) {
        if (node.type === "hidden_run") continue;
        const key = ruleOf(node).key;
        if (key && changed.has(key)) ids.add(node.id);
    }
    return ids;
}

export interface AgentViewControlsOptions {
    blockId: string;
    nodes: Accessor<readonly DocumentNode[]>;
    setDocumentState: (update: (prev: DocumentState) => DocumentState) => void;
}

export function createAgentViewControls(opts: AgentViewControlsOptions): AgentViewControls {
    const globalAtom = getSettingsKeyAtom(AGENT_VIEW_KEY as never) as () => unknown;
    const paneAtom = getBlockMetaKeyAtom(opts.blockId, AGENT_VIEW_KEY as never) as () => unknown;
    const resolved = createMemo(() => resolveView(globalAtom(), paneAtom()));
    const defaultView = createMemo(() => resolveView(globalAtom(), undefined).view);
    const [popoverOpen, setPopoverOpen] = createSignal(false);

    const writePane = (view: AgentView | null) => void setBlockMeta(opts.blockId, { [AGENT_VIEW_KEY]: view } as never);

    // A switch changed: the hand-made folds of the rows it governs go, so the switch does what it says.
    const apply = (next: AgentView | null) => {
        const before = resolved().view;
        const after = next ?? defaultView();
        const clear = foldsToClear(opts.nodes(), before, after);
        if (clear.size > 0) {
            opts.setDocumentState((prev) => {
                if (![...clear].some((id) => prev.collapsedNodes.has(id))) return prev;
                const collapsedNodes = new Set(prev.collapsedNodes);
                for (const id of clear) collapsedNodes.delete(id);
                return { ...prev, collapsedNodes };
            });
        }
        writePane(next);
    };

    return {
        resolved,
        defaultView,
        setPreset: (name) => apply({ ...VIEW_PRESETS[name] }),
        setSwitch: (key, value) => apply({ ...resolved().view, [key]: value }),
        followDefault: () => apply(null),
        makeDefault: () => {
            void RpcApi.SetConfigCommand(TabRpcClient, { [AGENT_VIEW_KEY]: resolved().view } as never);
            // The pane now says what the default says: follow it, so a later change of the default reaches it.
            writePane(null);
        },
        cycle: () => apply({ ...VIEW_PRESETS[nextPreset(resolved().view)] }),
        popoverOpen,
        setPopoverOpen,
    };
}

// ── The panes' controls, by block id ────────────────────────────────────────

const registry = new Map<string, AgentViewControls>();
const [registryVersion, bumpRegistry] = createSignal(0);

/** A mounted agent pane offers its controls; the returned function takes them back. */
export function registerAgentViewControls(blockId: string, controls: AgentViewControls): () => void {
    registry.set(blockId, controls);
    bumpRegistry((n) => n + 1);
    return () => {
        if (registry.get(blockId) === controls) registry.delete(blockId);
        bumpRegistry((n) => n + 1);
    };
}

/** The controls of the pane `blockId`, or undefined while it is not mounted. Reactive. */
export function agentViewControls(blockId: string | undefined): AgentViewControls | undefined {
    registryVersion();
    return blockId ? registry.get(blockId) : undefined;
}
