// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

/**
 * A reactive map of live objects by key: reads inside a memo or effect
 * re-run when an object registers or leaves.
 *
 * For a view module that must find its own models by block id (the
 * terminal's multi-input, a pane chrome reading its active tab's state)
 * without going through the host's view model, which for a native pane tab
 * is an adapter, not the module's own class (Pane Tab contract Phase 2c).
 */
export interface KeyedRegistry<T> {
    /** Registers `value` under `key`; returns the matching unregister, which
     *  leaves a later registration under the same key alone. */
    register(key: string, value: T): () => void;
    get(key: string): T | undefined;
    all(): T[];
}

export function createKeyedRegistry<T>(): KeyedRegistry<T> {
    const entries = new Map<string, T>();
    const [version, setVersion] = createSignal(0);
    return {
        register(key, value) {
            entries.set(key, value);
            setVersion((v) => v + 1);
            return () => {
                if (entries.get(key) !== value) return;
                entries.delete(key);
                setVersion((v) => v + 1);
            };
        },
        get(key) {
            version();
            return entries.get(key);
        },
        all() {
            version();
            return [...entries.values()];
        },
    };
}
