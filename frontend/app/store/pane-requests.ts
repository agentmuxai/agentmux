// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one way to ask the person something inside a pane, never the window
 * (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4.1,
 * Agent1's pane-modal spec, #18804, §3).
 *
 * `paneRequests.push(blockId, { source, Component, props })` adds a request to
 * that pane and returns `{ close }`. A `card` (the default) is drawn by the
 * agent pane's `PaneRequestDock`, above the composer; a `modal` is a multi-step
 * form the person finishes in place, drawn by `PaneRequestModals` inside the
 * pane's `<ModalLayer scope="pane">`, its component rendering a
 * `<Modal scope="pane">`. Either way the component gets its props and `close`.
 *
 * - **No window fallback.** A request for a pane that isn't on screen (another
 *   tab, a minimized window, a hibernated pane) stays keyed to it and shows
 *   when the pane does; meanwhile it is one source of the pane's "waiting for
 *   you" (`startWaitingForYou`), which gives the tone, the tab badge, the Swarm
 *   chip and the OS notification.
 * - **A closed pane gives the safe answer**: its requests go, and each one's
 *   `onGone` runs (deny, cancel).
 * - **Drawn from AgentMux's own record**: callers pass a component of their
 *   own and the text of srv's record, never an agent's words as a title or a
 *   button. The dock is marked operator-only, and every `Modal` a modal
 *   request renders is too (`OperatorOnlyModals`), so an agent's UI query,
 *   click and keys reach neither.
 */

import { createEffect, createRoot, createSignal, type JSX, untrack } from "solid-js";

import { endWaitingForYou, startWaitingForYou } from "@/app/notification/waiting-for-you";
import { MOS } from "@/app/store/global";

export type PaneRequestPresentation = "card" | "modal";

/** What every request component receives besides its own props. */
export interface PaneRequestProps {
    /** Takes this request away, answered. Idempotent. */
    close: () => void;
}

export interface PaneRequestOptions<P extends object> {
    /** What raised it, e.g. `vault-use` or `signin`; part of its "waiting for you" key. */
    source: string;
    Component: (props: P & PaneRequestProps) => JSX.Element;
    props?: P;
    /** `card` (default): docked above the agent pane's composer. `modal`: a form covering only the pane. */
    presentation?: PaneRequestPresentation;
    /** When it ends on its own, as the request's owner (srv) says: for a countdown. */
    deadlineMs?: number;
    /** What it asks, for the OS notification. */
    text?: string;
    /** What it wants in a few words, led by a verb ("asks to use a secret"), for the toast and the list. */
    what?: string;
    /** Its line in the dock's "N more" list while another is shown. */
    summary?: string;
    /** Whether something waits on the person for it (default true). */
    waits?: boolean;
    /** The pane was closed before it was answered: give the safe answer. */
    onGone?: () => void;
}

export interface PaneRequest {
    id: number;
    blockId: string;
    source: string;
    Component: (props: any) => JSX.Element;
    props: Record<string, unknown>;
    presentation: PaneRequestPresentation;
    deadlineMs?: number;
    summary?: string;
    waits: boolean;
}

export interface PaneRequestHandle {
    /** Takes the request away, answered. Idempotent. */
    close: () => void;
}

const [all, setAll] = createSignal<PaneRequest[]>([]);
/** Each request's watch on its pane, by id. */
const watches = new Map<number, () => void>();
let nextId = 1;

const waitKey = (r: PaneRequest): string => `pane-request:${r.source}:${r.id}`;

// `push` and `remove` never read the store tracked: a caller in an effect
// would otherwise re-run on its own push and push again.
function remove(id: number, reason: "submitted" | "closed"): PaneRequest | undefined {
    const r = untrack(all).find((x) => x.id === id);
    if (!r) return undefined;
    watches.get(id)?.();
    watches.delete(id);
    setAll((prev) => prev.filter((x) => x.id !== id));
    if (r.waits) endWaitingForYou(r.blockId, waitKey(r), reason);
    return r;
}

/** Ends `r` with the safe answer once its pane is deleted. A pane not loaded
 *  yet isn't gone: only one that was seen and then went is. */
function watchPane(r: PaneRequest, onGone: (() => void) | undefined): void {
    const dispose = createRoot((d) => {
        let seen = false;
        createEffect(() => {
            const present = !!MOS.getMuxObjectAtom<Block>(`block:${r.blockId}`)();
            if (present) seen = true;
            else if (seen) queueMicrotask(() => remove(r.id, "closed") && onGone?.());
        });
        return d;
    });
    watches.set(r.id, dispose);
}

export const paneRequests = {
    /** Ask the person something in pane `blockId`. */
    push<P extends object>(blockId: string, opts: PaneRequestOptions<P>): PaneRequestHandle {
        const r: PaneRequest = {
            id: nextId++,
            blockId,
            source: opts.source,
            Component: opts.Component as (props: any) => JSX.Element,
            props: { ...(opts.props ?? {}) } as Record<string, unknown>,
            presentation: opts.presentation ?? "card",
            deadlineMs: opts.deadlineMs,
            summary: opts.summary,
            waits: opts.waits ?? true,
        };
        setAll((prev) => [...prev, r]);
        untrack(() => {
            if (r.waits) startWaitingForYou(blockId, waitKey(r), opts.text, { what: opts.what });
            watchPane(r, opts.onGone);
        });
        return { close: () => void remove(r.id, "submitted") };
    },

    /** Pane `blockId`'s requests, in the order they came; reactive. */
    of(blockId: string): PaneRequest[] {
        return all().filter((r) => r.blockId === blockId);
    },

    /** Take request `id` away, answered. Idempotent. */
    close(id: number): void {
        remove(id, "submitted");
    },
};

/** A multi-step form for pane `blockId`, covering only that pane: sugar over `push` with `presentation: "modal"`. */
export function openPaneModal<P extends object>(
    blockId: string,
    Component: (props: P & PaneRequestProps) => JSX.Element,
    props?: P,
    opts: Omit<PaneRequestOptions<P>, "Component" | "props" | "presentation" | "source"> & { source?: string } = {}
): PaneRequestHandle {
    return paneRequests.push(blockId, { ...opts, source: opts.source ?? "modal", Component, props, presentation: "modal" });
}

/** Tests only: forget every request. */
export function __resetPaneRequests(): void {
    for (const r of untrack(all)) remove(r.id, "closed");
    nextId = 1;
}
