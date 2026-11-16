// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Show message" for one agent pane (the card, the panel's strip and rows): to the panel when the conversation has no
 * row for the message, else to its row, marked for a moment so a row already on screen still answers
 * (messages/show-message.ts).
 */

import type { DocumentNode } from "@/app/agent-model/node-types";
import { createSignal, onCleanup, type Accessor } from "solid-js";
import { messagesControlsFor } from "../messages/messages-state";
import { showMessage } from "../messages/show-message";

/** How long "Show message" marks the row it scrolled to. */
const MARK_MS = 2400;

export interface UseShowMessageOptions {
    blockId: string;
    /** The whole document, with the messages the panel holds. */
    document: Accessor<readonly DocumentNode[]>;
    /** The rows the conversation shows now, hidden-run lines included. */
    shown: Accessor<readonly DocumentNode[]>;
    /** Scroll the conversation to a message (a hidden run that holds it opens). */
    jumpTo: (nodeId: string) => void;
}

export function useShowMessage(opts: UseShowMessageOptions): { show: (nodeId: string) => void; marked: Accessor<string | null> } {
    const [marked, setMarked] = createSignal<string | null>(null);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const mark = (nodeId: string): void => {
        clearTimeout(timer);
        setMarked(nodeId);
        timer = setTimeout(() => setMarked(null), MARK_MS);
    };
    onCleanup(() => clearTimeout(timer));
    const hasRow = (nodeId: string): boolean =>
        opts.shown().some((n) => n.id === nodeId || (n.type === "hidden_run" && n.hiddenIds.includes(nodeId)));
    const show = (nodeId: string): void =>
        void showMessage(nodeId, {
            panel: messagesControlsFor(opts.blockId),
            nodes: opts.document,
            hasRow,
            jump: opts.jumpTo,
            mark,
        });
    return { show, marked };
}
