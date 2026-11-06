// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Messages panel's state for one agent pane (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md
 * §5.1, §5.6): kept in the pane's block meta, next to the view switches, so it follows the pane and survives a
 * restart. `enabled` is the per-pane setting (off until a person turns it on); `open` is the panel against the
 * rail; `seen` is the time up to which the person has seen the messages.
 */

import type { DocumentNode } from "@/app/agent-model/node-types";
import { getBlockMetaKeyAtom } from "@/app/store/block-atom-cache";
import { setBlockMeta } from "@/app/store/block-meta";
import { createContext, createMemo, createSignal, useContext, type Accessor } from "solid-js";

export const AGENT_MESSAGES_KEY = "agent:messages";

export interface MessagesState {
    enabled: boolean;
    open: boolean;
    seen: number;
}

export const DEFAULT_MESSAGES_STATE: MessagesState = { enabled: false, open: true, seen: 0 };

/** Meta is free-form JSON: take what is well-formed, default the rest. */
export function parseMessagesState(raw: unknown): MessagesState {
    if (!raw || typeof raw !== "object") return DEFAULT_MESSAGES_STATE;
    const r = raw as Record<string, unknown>;
    return {
        enabled: r.enabled === true,
        open: r.open !== false,
        seen: typeof r.seen === "number" && Number.isFinite(r.seen) && r.seen > 0 ? r.seen : 0,
    };
}

export interface MessagesPanelControls {
    state: Accessor<MessagesState>;
    patch: (changes: Partial<MessagesState>) => void;
    /** The message the person asked to see in the panel (a click on its stub in the conversation). */
    request: Accessor<{ nodeId: string; seq: number } | null>;
    /** The panel has acted on the request: forget it, so a panel that mounts again does not repeat it. */
    clearRequest: () => void;
    /** Open the panel on a message. */
    showInPanel: (nodeId: string) => void;
    /** Whether a row of the conversation is shown as a stub that opens the panel. */
    stubbed: (node: DocumentNode) => boolean;
}

export function createMessagesPanelControls(
    blockId: string,
    isForced: (node: DocumentNode) => boolean
): MessagesPanelControls {
    const atom = getBlockMetaKeyAtom(blockId, AGENT_MESSAGES_KEY as never) as () => unknown;
    const state = createMemo(() => parseMessagesState(atom()));
    const patch = (changes: Partial<MessagesState>) =>
        void setBlockMeta(blockId, { [AGENT_MESSAGES_KEY]: { ...state(), ...changes } } as never);
    const [request, setRequest] = createSignal<{ nodeId: string; seq: number } | null>(null);
    let seq = 0;
    return {
        state,
        patch,
        request,
        clearRequest: () => setRequest(null),
        showInPanel: (nodeId) => {
            if (!state().open) patch({ open: true });
            setRequest({ nodeId, seq: ++seq });
        },
        // Every incoming jekt that is not forced: a sensitive one, or one nothing proved, keeps its full row.
        stubbed: (node) =>
            state().enabled && node.type === "jekt_message" && node.direction === "incoming" && !isForced(node),
    };
}

/** What the conversation's rows need from the panel: provided by the pane's layout, absent outside it. */
export const MessagesPanelContext = createContext<MessagesPanelControls>();

export const useMessagesPanel = (): MessagesPanelControls | undefined => useContext(MessagesPanelContext);

/** Turn the panel on or off for a pane from outside it (the command palette), keeping its other state. */
export function setMessagesEnabled(blockId: string, enabled: boolean): void {
    const current = parseMessagesState((getBlockMetaKeyAtom(blockId, AGENT_MESSAGES_KEY as never) as () => unknown)());
    void setBlockMeta(blockId, {
        [AGENT_MESSAGES_KEY]: { ...current, enabled, open: enabled ? true : current.open },
    } as never);
}

/** Whether the panel is on for a pane. */
export function messagesEnabled(blockId: string): boolean {
    return parseMessagesState((getBlockMetaKeyAtom(blockId, AGENT_MESSAGES_KEY as never) as () => unknown)()).enabled;
}
