// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The order of the Swarm's agent rows: working agents on top, the rest by most recent
 * activity (docs/specs/SPEC_SWARM_ORDER_WORKING_FIRST_THEN_RECENT_2026_10_23.md).
 *
 * `orderSwarmItems` is the pure order. `SwarmOrderController` decides when the list
 * moves: a change waits for the settle time, and waits longer while the user is using
 * the list, so a row never jumps under the cursor.
 */

import { sameList } from "@/app/util/select-trace";
import { createSignal, type Accessor } from "solid-js";

/** How long the order waits after the last change, and after the user lets go of the list. */
export const SWARM_SETTLE_MS = 400;

/** The longest a right-click menu is assumed open when nothing else says it closed. */
const CONTEXT_MENU_HOLD_MS = 15_000;

/** One row of the list, with what the order reads from it. */
export interface SwarmOrderItem {
    /** The row's key, unique across local and remote rows. */
    key: string;
    /** A row of another AgentMux instance. */
    remote: boolean;
    /** The status chip says working or question (anything but idle). */
    working: boolean;
    /** When the agent last produced output, Unix ms; 0 when it never has, or a remote row (no value is shared). */
    lastActive: number;
    /** The agent's name, for ties. */
    name: string;
}

const byName = (a: string, b: string): number => a.localeCompare(b, undefined, { sensitivity: "base" });

/**
 * The keys in display order.
 *
 * Working rows first, then the rest. A local row comes before a remote one in its band;
 * remote rows keep the order they came in (by machine, then name). Working local rows go
 * newest-to-start-working first, by `workingSince` (not by last output, which changes about
 * once a second); the rest go by last activity, newest first, with agents that never
 * produced output last. Ties break on name, then key, so the order is total.
 */
export function orderSwarmItems(items: readonly SwarmOrderItem[], workingSince: ReadonlyMap<string, number>): string[] {
    const index = new Map(items.map((item, i) => [item.key, i]));
    const since = (item: SwarmOrderItem): number => workingSince.get(item.key) ?? item.lastActive;
    return [...items]
        .sort((a, b) => {
            if (a.working !== b.working) return a.working ? -1 : 1;
            if (a.remote !== b.remote) return a.remote ? 1 : -1;
            if (a.remote) return index.get(a.key)! - index.get(b.key)!;
            const recency = a.working ? since(b) - since(a) : b.lastActive - a.lastActive;
            return recency || byName(a.name, b.name) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
        })
        .map((item) => item.key);
}

/** Why the list is held where it is. */
export type SwarmHold = "pointer" | "press" | "menu" | "focus";

export interface SwarmOrderHooks {
    /** Called just before a new order is published, while the rows are still where they were. */
    beforeMove?: () => void;
    /** Called just after, once the rows are in their new places. */
    afterMove?: () => void;
    /**
     * Whether the page still shows the pointer over the list, or focus in it. The leave and
     * blur events that end those holds are not delivered when the element under the pointer or
     * focus is removed or disabled, so the controller asks before it trusts them.
     */
    stillHeld?: (hold: "pointer" | "focus") => boolean;
}

/**
 * Keeps the order the rows are shown in.
 *
 * `update` takes the facts each time they change. The new order is published after
 * `SWARM_SETTLE_MS` of no change, and not while the list is held (`setHold`): the pointer
 * is over it, a button is down in it, a right-click menu is open, or focus is inside a row.
 * It is published once the hold ends and the settle time has passed. Until then the rows
 * keep their places: an agent that leaves is removed at once, and one that joins is added at
 * the end. The very first list is shown at once.
 */
export class SwarmOrderController {
    readonly order: Accessor<string[]>;
    private readonly setOrder: (keys: string[]) => void;
    private readonly workingSince = new Map<string, number>();
    private readonly wasSeen = new Set<string>();
    private target: string[] = [];
    private timer: ReturnType<typeof setTimeout> | undefined;
    private menuTimer: ReturnType<typeof setTimeout> | undefined;
    private readonly holds = new Set<SwarmHold>();

    constructor(
        private readonly settleMs: number = SWARM_SETTLE_MS,
        private readonly now: () => number = Date.now,
        private readonly hooks: SwarmOrderHooks = {}
    ) {
        const [order, setOrder] = createSignal<string[]>([], { equals: sameList });
        this.order = order;
        this.setOrder = setOrder;
    }

    /** The rows and what the order reads from them, as they are now. */
    update(items: readonly SwarmOrderItem[]): void {
        this.dropStaleHolds();
        this.trackWorkingSince(items);
        const keys = new Set(items.map((item) => item.key));
        const target = orderSwarmItems(items, this.workingSince);
        const shown = this.order();
        if (shown.length === 0) {
            this.target = target;
            this.setOrder(target);
            return;
        }
        // Leavers go at once; joiners wait at the end for the next move.
        const kept = shown.filter((key) => keys.has(key));
        const shownKeys = new Set(kept);
        const next = [...kept, ...target.filter((key) => !shownKeys.has(key))];
        if (!sameList(next, shown)) this.setOrder(next);
        const targetChanged = !sameList(target, this.target);
        this.target = target;
        if (sameList(target, next)) {
            this.cancel();
        } else if (targetChanged || this.timer === undefined) {
            // Also when nothing is waiting: a hold the page no longer shows was just dropped.
            this.arm();
        }
    }

    /** Turn a reason for holding the list on or off. */
    setHold(hold: SwarmHold, on: boolean): void {
        if (on) this.holds.add(hold);
        else this.holds.delete(hold);
        if (hold === "menu") {
            clearTimeout(this.menuTimer);
            // A right-click menu says nothing when it closes, so it holds for a bounded time.
            if (on) this.menuTimer = setTimeout(() => this.setHold("menu", false), CONTEXT_MENU_HOLD_MS);
        }
        if (this.holds.size === 0 && !sameList(this.target, this.order())) this.arm();
    }

    get held(): boolean {
        this.dropStaleHolds();
        return this.holds.size > 0;
    }

    /** Let go of a pointer or focus hold the page no longer shows (see `SwarmOrderHooks.stillHeld`). */
    private dropStaleHolds(): void {
        for (const hold of ["pointer", "focus"] as const) {
            if (this.holds.has(hold) && this.hooks.stillHeld?.(hold) === false) this.holds.delete(hold);
        }
    }

    /** Let go of every hold that comes from the list element itself, e.g. because it was removed. */
    releaseListHolds(): void {
        for (const hold of ["pointer", "press", "focus"] as const) this.holds.delete(hold);
        if (this.holds.size === 0 && !sameList(this.target, this.order())) this.arm();
    }

    dispose(): void {
        this.cancel();
        clearTimeout(this.menuTimer);
    }

    private cancel(): void {
        clearTimeout(this.timer);
        this.timer = undefined;
    }

    private arm(): void {
        this.cancel();
        if (this.held) return;
        this.timer = setTimeout(() => {
            this.timer = undefined;
            if (this.held) return;
            const shown = this.order();
            const keys = new Set(shown);
            const next = this.target.filter((key) => keys.has(key));
            if (sameList(next, shown)) return;
            this.hooks.beforeMove?.();
            this.setOrder(next);
            this.hooks.afterMove?.();
        }, this.settleMs);
    }

    /**
     * An agent that starts working takes the top of the working rows. The time it started is
     * kept while it stays in the working band; an agent first seen already working is dated by
     * its last output.
     */
    private trackWorkingSince(items: readonly SwarmOrderItem[]): void {
        const live = new Set<string>();
        for (const item of items) {
            if (item.remote) continue;
            live.add(item.key);
            if (item.working) {
                if (!this.workingSince.has(item.key)) {
                    this.workingSince.set(
                        item.key,
                        this.wasSeen.has(item.key) ? this.now() : item.lastActive || this.now()
                    );
                }
            } else {
                this.workingSince.delete(item.key);
            }
            this.wasSeen.add(item.key);
        }
        for (const key of [...this.wasSeen]) {
            if (!live.has(key)) {
                this.wasSeen.delete(key);
                this.workingSince.delete(key);
            }
        }
    }
}
