// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Command types already reported as unknown, per handler map, so a stream of them logs one line each. */
const reported = new WeakMap<object, Set<string>>();

/**
 * The handler for a command type in a reducer's `HANDLERS` map, or undefined for a
 * type the map has no entry for. An older frontend can meet a command from a newer
 * srv: the caller then returns its state unchanged, and this logs the type once.
 * The lookup uses `Object.hasOwn`, so a name from `Object.prototype` ("constructor",
 * "toString") is not taken for a handler.
 */
export function pickHandler<H extends object>(label: string, handlers: H, type: string): H[keyof H] | undefined {
    if (Object.hasOwn(handlers, type)) {
        return (handlers as Record<string, H[keyof H]>)[type];
    }
    let seen = reported.get(handlers);
    if (!seen) {
        seen = new Set();
        reported.set(handlers, seen);
    }
    if (!seen.has(type)) {
        seen.add(type);
        console.warn(`[${label}] ignoring unknown command type "${type}"`);
    }
    return undefined;
}
