// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: jotaiStore replaced by a module-level reactive root.
// The "globalStore" shim below is kept for call-sites that still use
// globalStore.get / globalStore.set during the incremental migration.
// As each file is ported to SolidJS signals the usages here will vanish.

import { createRoot, getOwner } from "solid-js";

// Persistent reactive owner for module-level createMemo / createEffect calls
// that live outside a component tree.
let _reactiveOwner: ReturnType<typeof getOwner> | null = null;
let _disposeRoot: (() => void) | null = null;

export function initReactiveRoot() {
    if (_reactiveOwner != null) return;
    _disposeRoot = createRoot((dispose) => {
        _reactiveOwner = getOwner();
        return dispose;
    });
}

export function getReactiveOwner() {
    return _reactiveOwner;
}

// Thin compatibility shim used by files not yet migrated to signals.
// get(signal) calls signal(), set(setter, value) calls setter(value).
export const globalStore = {
    get<T>(signalOrAccessor: (() => T) | any): T {
        if (typeof signalOrAccessor === "function") {
            return (signalOrAccessor as () => T)();
        }
        console.warn("[globalStore.get] called with non-function:", signalOrAccessor);
        return undefined as unknown as T;
    },
    set<T>(setter: ((v: T | ((prev: T) => T)) => void) | any, value: T | ((prev: T) => T)) {
        if (typeof setter === "function") {
            (setter as (v: T | ((prev: T) => T)) => void)(value);
        } else {
            console.warn("[globalStore.set] called with non-function setter:", setter);
        }
    },
};
