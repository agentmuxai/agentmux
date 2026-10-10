// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The widget panes open in this window, and what their status bar items show
 * now (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.8).
 *
 * A palette command or a status item click is delivered to a pane of its
 * widget (`widget-contributions.tsx`); this is where it finds one, or waits
 * for the one it opened. A pane's `ui.setStatusItem` look lasts while that
 * pane is open; when it closes, the item shows its manifest's look again.
 */

import { createSignal } from "solid-js";

export type CommandSource = "palette" | "status";

export interface WidgetPane {
    pkgId: string;
    view: string;
    /** Delivers a command (the `command` event); a pane not yet connected
     *  keeps it until it is. Absent for a trusted widget without `command`. */
    command?: (id: string, source: CommandSource) => void;
}

const panes = new Map<string, WidgetPane>();
const waiters = new Map<string, ((p: WidgetPane) => void)[]>();

/** Notes that `blockId` shows `pane`; returns what forgets it, and the
 *  status item looks it set. */
export function trackWidgetPane(blockId: string, pane: WidgetPane): () => void {
    panes.set(blockId, pane);
    for (const resolve of waiters.get(blockId) ?? []) resolve(pane);
    waiters.delete(blockId);
    return () => {
        if (panes.get(blockId) !== pane) return;
        panes.delete(blockId);
        clearStatusLooksOf(blockId);
    };
}

/** The open panes of `view`, as `[blockId, pane]`. */
export function widgetPanesOf(view: string): [string, WidgetPane][] {
    return [...panes].filter(([, p]) => p.view === view);
}

/** The pane in `blockId` once it registers; null after `timeoutMs`. */
export function waitForWidgetPane(blockId: string, timeoutMs: number): Promise<WidgetPane | null> {
    const now = panes.get(blockId);
    if (now) return Promise.resolve(now);
    return new Promise((resolve) => {
        const done = (p: WidgetPane | null) => {
            clearTimeout(timer);
            const list = waiters.get(blockId)?.filter((w) => w !== done);
            if (list?.length) waiters.set(blockId, list);
            else waiters.delete(blockId);
            resolve(p);
        };
        const timer = setTimeout(() => done(null), timeoutMs);
        waiters.set(blockId, [...(waiters.get(blockId) ?? []), done]);
    });
}

// ── Status item looks ───────────────────────────────────────────────────────

export type StatusTone = "info" | "success" | "warning" | "error";

/** What a pane set with `ui.setStatusItem`; a field left out shows the
 *  manifest's. */
export interface StatusLook {
    text?: string;
    icon?: string;
    tooltip?: string;
    tone?: StatusTone;
    hidden?: boolean;
}

const [looks, setLooks] = createSignal<Record<string, { look: StatusLook; blockId: string }>>({});

const lookKey = (pkgId: string, itemId: string) => `${pkgId}/${itemId}`;

/** Sets the look of item `itemId` of `pkgId`, from the pane `blockId`;
 *  `null` puts the manifest's back. */
export function setStatusLook(pkgId: string, itemId: string, blockId: string, look: StatusLook | null): void {
    setLooks((prev) => {
        const next = { ...prev };
        if (look) next[lookKey(pkgId, itemId)] = { look, blockId };
        else delete next[lookKey(pkgId, itemId)];
        return next;
    });
}

/** The look a pane set for the item, if any. Reactive. */
export function statusLook(pkgId: string, itemId: string): StatusLook | undefined {
    return looks()[lookKey(pkgId, itemId)]?.look;
}

function clearStatusLooksOf(blockId: string): void {
    if (!Object.values(looks()).some((l) => l.blockId === blockId)) return;
    setLooks((prev) => Object.fromEntries(Object.entries(prev).filter(([, l]) => l.blockId !== blockId)));
}

/** Tests only. */
export function __resetWidgetPanes(): void {
    panes.clear();
    waiters.clear();
    setLooks({});
}
