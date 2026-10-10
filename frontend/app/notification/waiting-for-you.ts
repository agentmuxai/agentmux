// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Everything an agent is waiting on the user for, in this window
 * (docs/reports/REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §2).
 *
 * Each source of a call to action says when it starts and ends waiting:
 * - an AskUserQuestion (`useAgentQuestions`);
 * - a tool permission (`useAgentDecisions`);
 * - and, through srv's `userattention` event, a browser hand-off or
 *   approval, an SSH consent and a widget install.
 *
 * A pane is waiting while any of its sources is. On its first source the
 * pane's `waiting-for-input` event fires, which loops the waiting tone (the
 * sound service), shows the OS notification and badges the taskbar (the OS
 * bridge); on its last, `waiting-ended`.
 *
 * Requests that belong to no pane are kept under an `app:` key. They play the
 * tone only while this window isn't in front, and send no OS notification.
 */

import { createEffect, createRoot } from "solid-js";
import { fireEvent as firePaneEvent } from "@/app/store/agent-pane-state-store";
import { MOS } from "@/app/store/global";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { windowId } from "@/app/store/window-identity";

import { APP_KEY_PREFIX } from "./waiting-keys";

export { APP_KEY_PREFIX };

const waiting = new Map<string, Map<string, string | undefined>>();
/** `blockId\0source` → disposes the watch that ends that wait on its own. */
const watches = new Map<string, () => void>();

export interface WaitingOptions {
    /** How many questions it asks, for the notification's "(+N more)". */
    questionCount?: number;
    /**
     * Whether it still waits, read reactively. The registry ends the wait
     * when this turns false or the pane is deleted, whether or not the pane
     * is mounted, so a wait outlives a tab switch but not its pane.
     */
    stillWaiting?: () => boolean;
}

/** `source` (e.g. "question", "permission", "srv:<key>") now waits on the
 *  user in pane `blockId`; `text` says what it wants, for the notification. */
export function startWaitingForYou(blockId: string, source: string, text?: string, opts: WaitingOptions = {}): void {
    if (!blockId) return;
    let sources = waiting.get(blockId);
    if (!sources) {
        sources = new Map();
        waiting.set(blockId, sources);
    }
    const first = sources.size === 0;
    sources.set(source, text);
    if (opts.stillWaiting) watch(blockId, source, opts.stillWaiting);
    if (first) firePaneEvent(blockId, { type: "waiting-for-input", question: text, questionCount: opts.questionCount });
}

function watch(blockId: string, source: string, stillWaiting: () => boolean): void {
    const key = `${blockId}\0${source}`;
    if (watches.has(key)) return;
    const dispose = createRoot((d) => {
        createEffect(() => {
            const gone = !MOS.getMuxObjectAtom<Block>(`block:${blockId}`)();
            if (gone || !stillWaiting()) queueMicrotask(() => endWaitingForYou(blockId, source, gone ? "closed" : "submitted"));
        });
        return d;
    });
    watches.set(key, dispose);
}

/** `source` no longer waits in pane `blockId`: `submitted` when the user
 *  answered, `closed` when it went away unanswered. */
export function endWaitingForYou(blockId: string, source: string, reason: "submitted" | "closed" = "submitted"): void {
    const key = `${blockId}\0${source}`;
    watches.get(key)?.();
    watches.delete(key);
    const sources = waiting.get(blockId);
    if (!sources?.delete(source)) return;
    if (sources.size > 0) return;
    waiting.delete(blockId);
    firePaneEvent(blockId, { type: "waiting-ended", reason });
}

export function isWaitingForYou(blockId: string): boolean {
    return (waiting.get(blockId)?.size ?? 0) > 0;
}

/** srv's announcement of a call to action it holds open (`userattention`,
 *  crates/srv/src/backend/user_attention.rs). */
export interface UserAttention {
    key: string;
    block_id: string;
    kind: string;
    text: string;
    window_ids: string[];
    active: boolean;
}

/** Apply one of srv's announcements, if it's for this window: one that
 *  names windows is for those; one that names none is for every window. */
export function applyUserAttention(a: UserAttention, thisWindow: string): void {
    if (!a?.key) return;
    if (a.window_ids?.length && !a.window_ids.includes(thisWindow)) return;
    const blockId = a.block_id || `${APP_KEY_PREFIX}${a.key}`;
    if (a.active) startWaitingForYou(blockId, `srv:${a.key}`, a.text);
    else endWaitingForYou(blockId, `srv:${a.key}`, "submitted");
}

let installed = false;

/** Follow srv's announcements. Call once per window. */
export function installWaitingForYou(): void {
    if (installed) return;
    installed = true;
    muxEventSubscribe({
        eventType: WpsEvent.UserAttention,
        handler: (event: { data?: UserAttention }) => {
            if (event?.data) applyUserAttention(event.data, windowId());
        },
    });
}

/** Test helper. */
export function __resetWaitingForYou(): void {
    for (const d of watches.values()) d();
    watches.clear();
    waiting.clear();
}
