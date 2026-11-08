// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Messages panel's state for one agent pane (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md
 * §5.1, §5.6, and its refinements for #28993): the setting and the time the person last saw the messages are kept in
 * the pane's block meta, next to the view switches, so they follow the pane and survive a restart. `enabled` is the
 * per-pane setting (on until a person hides it); `seen` is the time up to which the person has seen the messages.
 * `open` is the panel over the conversation: this session only, so a restart never covers the conversation.
 */

import type { DocumentNode } from "@/app/agent-model/node-types";
import { getBlockMetaKeyAtom } from "@/app/store/block-atom-cache";
import { setBlockMeta } from "@/app/store/block-meta";
import { createContext, createMemo, createSignal, getOwner, onCleanup, useContext, type Accessor } from "solid-js";

export const AGENT_MESSAGES_KEY = "agent:messages";

export interface MessagesState {
    enabled: boolean;
    open: boolean;
    seen: number;
}

/** On, with the panel collapsed to its bar. */
export const DEFAULT_MESSAGES_STATE: MessagesState = { enabled: true, open: false, seen: 0 };

/** Meta is free-form JSON: take what is well-formed, default the rest. `open` is never stored. */
export function parseMessagesState(raw: unknown): MessagesState {
    if (!raw || typeof raw !== "object") return DEFAULT_MESSAGES_STATE;
    const r = raw as Record<string, unknown>;
    return {
        enabled: r.enabled !== false,
        open: false,
        seen: typeof r.seen === "number" && Number.isFinite(r.seen) && r.seen > 0 ? r.seen : 0,
    };
}

/** Whether `node` is an incoming jekt that lives in the panel only: not a forced one (a sensitive jekt, or one nothing proved), which keeps its row. */
export function isPanelOnlyJekt(node: DocumentNode, enabled: boolean, isForced: (node: DocumentNode) => boolean): boolean {
    return enabled && node.type === "jekt_message" && node.direction === "incoming" && !isForced(node);
}

/** `nodes` without the incoming jekts the Messages panel holds, while it is on for the pane. */
export function withoutPanelJekts(
    nodes: DocumentNode[],
    blockId: string,
    isForced: (node: DocumentNode) => boolean
): DocumentNode[] {
    return messagesEnabled(blockId) ? nodes.filter((n) => !isPanelOnlyJekt(n, true, isForced)) : nodes;
}

export interface MessagesPanelControls {
    state: Accessor<MessagesState>;
    /** Change the setting or `seen` (kept in the pane's meta), or `open` (this session). */
    patch: (changes: Partial<MessagesState>) => void;
    /** The message the person asked to see in the panel (a search match inside a message the conversation does not show). */
    request: Accessor<{ nodeId: string; seq: number } | null>;
    /** The panel has acted on the request: forget it, so a panel that mounts again does not repeat it. */
    clearRequest: () => void;
    /** Open the panel on a message. */
    showInPanel: (nodeId: string) => void;
    /** Whether the conversation leaves `node` out because the panel holds it. */
    hidden: (node: DocumentNode) => boolean;
}

export function createMessagesPanelControls(
    blockId: string,
    isForced: (node: DocumentNode) => boolean
): MessagesPanelControls {
    const atom = getBlockMetaKeyAtom(blockId, AGENT_MESSAGES_KEY as never) as () => unknown;
    const [open, setOpen] = createSignal(false);
    const stored = createMemo(() => parseMessagesState(atom()));
    const state = createMemo<MessagesState>(() => ({ ...stored(), open: open() }));
    const patch = (changes: Partial<MessagesState>) => {
        const { open: opened, ...kept } = changes;
        if (opened !== undefined) setOpen(opened);
        if (Object.keys(kept).length === 0) return;
        const { enabled, seen } = { ...stored(), ...kept };
        void setBlockMeta(blockId, { [AGENT_MESSAGES_KEY]: { enabled, seen } } as never);
    };
    const [request, setRequest] = createSignal<{ nodeId: string; seq: number } | null>(null);
    let seq = 0;
    const controls: MessagesPanelControls = {
        state,
        patch,
        request,
        clearRequest: () => setRequest(null),
        showInPanel: (nodeId) => {
            setOpen(true);
            setRequest({ nodeId, seq: ++seq });
        },
        hidden: (node) => isPanelOnlyJekt(node, stored().enabled, isForced),
    };
    byBlock.set(blockId, controls);
    if (getOwner()) onCleanup(() => byBlock.get(blockId) === controls && byBlock.delete(blockId));
    return controls;
}

/** The panel controls of each pane that has them, for what acts on a pane from outside it (the palette, search). */
const byBlock = new Map<string, MessagesPanelControls>();

export const messagesControlsFor = (blockId: string): MessagesPanelControls | undefined => byBlock.get(blockId);

/**
 * Show `nodeId` in the panel when the conversation leaves it out (a search match inside such a message has no row
 * to jump to). False when it is a row of the conversation, or the pane has no panel.
 */
export function showInPanelIfHeld(blockId: string, nodes: readonly DocumentNode[], nodeId: string): boolean {
    const panel = byBlock.get(blockId);
    const node = panel && nodes.find((n) => n.id === nodeId);
    if (!panel || !node || !panel.hidden(node)) return false;
    panel.showInPanel(nodeId);
    return true;
}

/** What the conversation's rows need from the panel: provided by the pane's layout, absent outside it. */
export const MessagesPanelContext = createContext<MessagesPanelControls>();

export const useMessagesPanel = (): MessagesPanelControls | undefined => useContext(MessagesPanelContext);

/** Turn the panel on or off for a pane from outside it (the command palette), keeping its other state. */
export function setMessagesEnabled(blockId: string, enabled: boolean): void {
    const current = parseMessagesState((getBlockMetaKeyAtom(blockId, AGENT_MESSAGES_KEY as never) as () => unknown)());
    void setBlockMeta(blockId, { [AGENT_MESSAGES_KEY]: { enabled, seen: current.seen } } as never);
}

/** Whether the panel is on for a pane. */
export function messagesEnabled(blockId: string): boolean {
    return parseMessagesState((getBlockMetaKeyAtom(blockId, AGENT_MESSAGES_KEY as never) as () => unknown)()).enabled;
}
