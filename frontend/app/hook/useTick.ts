// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type Accessor, createSignal, onCleanup } from "solid-js";

interface TickerEntry {
    tick: Accessor<number>;
    refCount: number;
    id: ReturnType<typeof setInterval>;
}

// One setInterval per distinct period. Ref-counted so the interval is cleared
// when the last subscriber unmounts.
const tickers = new Map<number, TickerEntry>();

/**
 * Returns a reactive counter that increments every `ms` milliseconds.
 * All callers with the same period share one underlying setInterval.
 * Automatically cleans up when the last subscriber's reactive scope disposes.
 *
 * Usage — always-on tick:
 *   const tick = useTick(1000);
 *   const now = createMemo(() => (tick(), Date.now()));
 *
 * Usage — gated via short-circuit (natural subscription pruning):
 *   const elapsed = createMemo(() => {
 *       const end = node.exitedAt ?? (tick(), Date.now());
 *       return formatElapsed(end - node.startedAt);
 *   });
 */
export function useTick(ms: number): Accessor<number> {
    let entry = tickers.get(ms);
    if (!entry) {
        const [tick, setTick] = createSignal(0);
        const id = setInterval(() => setTick((n) => n + 1), ms);
        entry = { tick, refCount: 0, id };
        tickers.set(ms, entry);
    }
    entry.refCount++;
    const captured = entry;
    onCleanup(() => {
        captured.refCount--;
        if (captured.refCount === 0) {
            clearInterval(captured.id);
            tickers.delete(ms);
        }
    });
    return captured.tick;
}
