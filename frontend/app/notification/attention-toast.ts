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

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { isLightThemeActive } from "@/app/block/pane-identity";
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
/** A request that leaves the list and returns within this long is not new. */
export const RETURN_MS = 5000;

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
    /** An agent's colour as readable text on the toast in the current theme; absent, the colour as given. */
    tint?: (hex?: string) => string | undefined;
    now: () => number;
    graceMs?: number;
}

const goKey = (blockId: string) => `attention:go:${blockId}`;

/** The toast for `items`, which are not empty. A pure function of the list, for tests. */
export function buildToast(
    items: AttentionItem[],
    reveal: (blockId: string) => void,
    setAction: AttentionToastDeps["setAction"],
    tint: (hex?: string) => string | undefined = (hex) => hex
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
            titleColor: tint(i.color),
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
    // When each request was last in the list this toast considers. One that leaves and comes back within
    // `RETURN_MS` (srv's rows flicker while another window reveals a pane) is the same request, not a new one.
    const known = new Map<string, number>();
    // Closed with its X: those requests do not bring it back.
    let dismissed = new Set<string>();
    let shown = false;
    // What the toast on screen says. The list re-reads on every agent event, and a toast pushed again, unchanged,
    // would be read out again by a screen reader.
    let said = "";
    const hide = () => {
        if (shown) deps.remove(ATTENTION_TOAST_ID);
        shown = false;
        said = "";
    };
    return {
        update() {
            const now = deps.now();
            const all = deps.items();
            dismissed = new Set([...dismissed].filter((id) => all.some((i) => i.blockId === id)));
            // The pane the person is looking at in the front window shows its own request: it is not considered, so
            // it is new when they look away.
            const focused = deps.inFront() ? deps.focusedBlock() : null;
            const items = all.filter((i) => i.blockId !== focused && !dismissed.has(i.blockId));
            const fresh = items.filter((i) => !known.has(i.blockId));
            for (const i of items) known.set(i.blockId, now);
            for (const [id, last] of known)
                if (!items.some((i) => i.blockId === id) && now - last > RETURN_MS) known.delete(id);
            if (items.length === 0 || !deps.enabled()) return hide();
            const isNew = fresh.length > 0 && now - startedAt >= (deps.graceMs ?? START_GRACE_MS);
            // A window that is not in front starts none (the OS notification is the signal) but leaves the one it has:
            // the request it names still waits.
            if (!(isNew && deps.inFront()) && !shown) return;
            const says = JSON.stringify(items.map((i) => [i.blockId, i.name, i.color, i.what, i.local]));
            if (shown && says === said) return;
            said = says;
            // Its X and any of its buttons take the toast off the screen: what it named does not bring it back.
            const close = () => {
                shown = false;
                said = "";
                for (const i of items) dismissed.add(i.blockId);
            };
            const toast = buildToast(
                items,
                (blockId) => {
                    close();
                    deps.reveal(blockId);
                },
                deps.setAction,
                deps.tint
            );
            toast.onClose = close;
            shown = true;
            deps.push({ ...toast, timestamp: new Date(now).toISOString() });
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
            tint: (hex) => paneRoleColor(undefined, hex, isLightThemeActive(), "identity"),
            now: Date.now,
        });
        createEffect(() => toast.update());
    });
}
