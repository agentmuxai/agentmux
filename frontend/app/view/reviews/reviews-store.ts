// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The list the Reviews pane shows, kept up to date: `GET /api/v1/reviews` once on open, again when srv says a
 * review, checks, merge or task event happened (a short wait gathers a burst into one read), and every minute
 * while the pane is open as the fallback. srv builds the list from its own memory and the board, so a read costs
 * no GitHub call; the pane never reads GitHub itself (docs/specs/SPEC_REVIEWS_PANE_2026_11_15.md §6.1).
 */

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { ReviewsList } from "@/types/rpc/ReviewsList";
import { createSignal, onCleanup, type Accessor } from "solid-js";
import { srvJson } from "../atlas/atlas-srv";

/** How long a burst of events waits before one read. */
export const REFRESH_WAIT_MS = 1500;
/** The fallback read while the pane is open. */
export const POLL_MS = 60_000;

export interface ReviewsData {
    /** The newest list; null before the first answer. */
    list: Accessor<ReviewsList | null>;
    /** `loading` until the first read ends; `unavailable` when srv could not answer (older than the route, or out of reach). */
    status: Accessor<"loading" | "ready" | "unavailable">;
    /** Reads now. */
    refresh: () => void;
    /** The window's clock in ms, ticking each minute so ages stay true. */
    now: Accessor<number>;
}

/** The list, following srv for as long as the caller lives. */
export function useReviews(): ReviewsData {
    const [list, setList] = createSignal<ReviewsList | null>(null);
    const [status, setStatus] = createSignal<"loading" | "ready" | "unavailable">("loading");
    const [now, setNow] = createSignal(Date.now());
    let wait: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    let reading = false;
    let again = false;

    const read = async () => {
        if (reading) {
            again = true;
            return;
        }
        reading = true;
        const next = await srvJson<ReviewsList>("/api/v1/reviews");
        reading = false;
        if (disposed) return;
        setNow(Date.now());
        if (next && Array.isArray(next.rows)) {
            setList(next);
            setStatus("ready");
        } else if (list() === null) {
            setStatus("unavailable");
        }
        if (again) {
            again = false;
            void read();
        }
    };
    const soon = () => {
        clearTimeout(wait);
        wait = setTimeout(() => void read(), REFRESH_WAIT_MS);
    };

    const off = [
        onMuxEvent(WpsEvent.ReviewFlow, soon),
        onMuxEvent(WpsEvent.ChecksFlow, soon),
        onMuxEvent(WpsEvent.MergeFlow, soon),
        onMuxEvent(WpsEvent.TaskFlow, soon),
    ];
    const poll = setInterval(() => void read(), POLL_MS);
    void read();
    onCleanup(() => {
        disposed = true;
        clearTimeout(wait);
        clearInterval(poll);
        for (const stop of off) stop();
    });
    return { list, status, refresh: () => void read(), now };
}
