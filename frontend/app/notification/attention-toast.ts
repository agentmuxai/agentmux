// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The toast that says who needs the person and takes them there
 * (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.2, §3.4, §3.5).
 *
 * One toast for the whole list, rewritten in place: an agent's name in its colour, what it wants, and a "Go to
 * agent" button per agent (at most five). It appears when an item enters the list, in the window that is in
 * front and only there (the OS notification is the signal when none is), and not for what was already waiting
 * when the window opened. It never takes focus. It clears itself when the list empties, and closing it with its
 * X does not answer anything and does not bring it back; a request that comes after does.
 */

import { pushNotification, removeNotificationById } from "@/app/store/flash-notifications";
import { focusManager } from "@/app/store/focusManager";
import { getSettingsKeyAtom } from "@/app/store/global";
import { revealBlock } from "@/app/util/reveal-block";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import { createEffect, createRoot } from "solid-js";
import { attentionItems, type AttentionItem } from "./attention-list";
import { setNotificationAction } from "./usenotification";

export const ATTENTION_TOAST_ID = "attention:waiting";
/** The most agents the toast links to; the rest are counted. */
export const TOAST_MAX_LINKS = 5;
/** What was already waiting this soon after the window opened is not new: srv's rows arrive a moment late. */
export const START_GRACE_MS = 4000;

export interface AttentionToastDeps {
    items: () => AttentionItem[];
    /** The setting `notify:waiting:toast` (default on). */
    enabled: () => boolean;
    /** Whether this window is the one in front. */
    inFront: () => boolean;
    /** The pane the person has focused in this window. */
    focusedBlock: () => string | null;
    push: (n: NotificationType) => void;
    remove: (id: string) => void;
    /** Registers what a toast button runs. */
    setAction: (key: string, fn: () => void) => void;
    reveal: (blockId: string) => void;
    now: () => number;
    graceMs?: number;
}

const goKey = (blockId: string) => `attention:go:${blockId}`;

/** The toast for `items`, which are not empty. A pure function of the list, for tests. */
export function buildToast(
    items: AttentionItem[],
    reveal: (blockId: string) => void,
    setAction: AttentionToastDeps["setAction"]
): NotificationType {
    const shown = items.slice(0, TOAST_MAX_LINKS);
    for (const i of shown) setAction(goKey(i.blockId), () => reveal(i.blockId));
    const where = (i: AttentionItem) => (i.local ? "" : " (in another window)");
    if (items.length === 1) {
        const [i] = items;
        return {
            id: ATTENTION_TOAST_ID,
            icon: "bell",
            type: "warning",
            persistent: true,
            title: i.name,
            titleColor: i.color,
            message: `${i.what}${where(i)}`,
            timestamp: "",
            live: "status",
            actions: [{ label: "Go to agent", actionKey: goKey(i.blockId) }],
        };
    }
    const more = items.length - shown.length;
    const lines = shown.map((i) => `${i.name} ${i.what}${where(i)}`);
    if (more > 0) lines.push(`and ${more} more`);
    return {
        id: ATTENTION_TOAST_ID,
        icon: "bell",
        type: "warning",
        persistent: true,
        title: `${items.length} agents need you`,
        message: lines.join("\n"),
        timestamp: "",
        live: "status",
        actions: shown.map((i) => ({ label: `Go to ${i.name}`, actionKey: goKey(i.blockId) })),
    };
}

/** The toast's controller: call `update()` whenever the list, the setting or the window's focus changes. */
export function createAttentionToast(deps: AttentionToastDeps): { update: () => void } {
    const startedAt = deps.now();
    let known = new Set<string>();
    // Closed with its X: those requests do not bring it back.
    let dismissed = new Set<string>();
    let shown = false;
    return {
        update() {
            const all = deps.items();
            const ids = new Set(all.map((i) => i.blockId));
            const fresh = [...ids].filter((id) => !known.has(id) && !dismissed.has(id));
            const inGrace = deps.now() - startedAt < (deps.graceMs ?? START_GRACE_MS);
            known = ids;
            dismissed = new Set([...dismissed].filter((id) => ids.has(id)));
            const focused = deps.inFront() ? deps.focusedBlock() : null;
            // The pane the person is looking at in the front window shows its own request.
            const items = all.filter((i) => !dismissed.has(i.blockId) && i.blockId !== focused);
            if (items.length === 0) {
                if (shown) deps.remove(ATTENTION_TOAST_ID);
                shown = false;
                return;
            }
            if (!deps.enabled()) {
                // Turned off while it shows: it goes.
                if (shown) deps.remove(ATTENTION_TOAST_ID);
                shown = false;
                return;
            }
            const isNew = fresh.length > 0 && !inGrace;
            // A window that is not in front starts none (the OS notification is the signal) but leaves the one it has:
            // the request it names still waits.
            if (!(isNew && deps.inFront()) && !shown) return;
            const toast = buildToast(items, deps.reveal, deps.setAction);
            toast.onClose = () => {
                shown = false;
                for (const i of items) dismissed.add(i.blockId);
            };
            shown = true;
            deps.push({ ...toast, timestamp: new Date(deps.now()).toISOString() });
        },
    };
}

let installed = false;

/** Show the attention toast in this window. Call once per window. */
export function installAttentionToast(): void {
    if (installed) return;
    installed = true;
    createRoot(() => {
        const inFront = makeWindowFocusSignal();
        const enabledAtom = getSettingsKeyAtom("notify:waiting:toast");
        const toast = createAttentionToast({
            items: attentionItems,
            enabled: () => enabledAtom() !== false,
            inFront,
            focusedBlock: focusManager.blockFocusAtom,
            push: pushNotification,
            remove: removeNotificationById,
            setAction: setNotificationAction,
            reveal: (blockId) => void revealBlock(blockId, { source: "attention" }),
            now: Date.now,
        });
        createEffect(() => toast.update());
    });
}
