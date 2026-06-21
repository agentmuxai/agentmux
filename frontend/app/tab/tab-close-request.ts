// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Thin bridge so keymodel.ts can trigger the tab-close confirmation flow
// without importing from tabbar.tsx (which would create a circular dep via
// the store imports). TabBar registers its requestClose handler on mount;
// callers (keyboard shortcuts, etc.) call triggerTabCloseRequest().

let _handler: (() => void) | null = null;

export function registerTabCloseRequestHandler(fn: () => void): () => void {
    _handler = fn;
    return () => {
        if (_handler === fn) _handler = null;
    };
}

export function triggerTabCloseRequest(): void {
    _handler?.();
}
