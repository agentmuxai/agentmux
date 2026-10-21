// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Opt-in debug lines, off by default, as the `debug` package's were: set
 * `localStorage.debug` to a comma-separated list of namespaces, `*` matching
 * anything (`"wave:*"`, `"wave:ws,wave:router"`), then reload. Unlike
 * `Logger.debug`, nothing reaches the host log: these lines can come once per
 * RPC message or focus change.
 * SPEC_MARKDOWN_CODE_ON_SHIKI_AND_REDUNDANT_LIBRARIES_2026_10_21.md §6.2.
 */

let patterns: RegExp[] | undefined;

function enabledPatterns(): RegExp[] {
    if (patterns) return patterns;
    let setting: string | null = null;
    try {
        setting = globalThis.localStorage?.getItem("debug") ?? null;
    } catch {
        // No storage here: debug lines stay off.
    }
    patterns = (setting ?? "")
        .split(/[\s,]+/)
        .filter(Boolean)
        .map((p) => new RegExp(`^${p.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*")}$`));
    return patterns;
}

/** A logger for `namespace` that prints only when `localStorage.debug` names it. */
export function debugLog(namespace: string): (...args: unknown[]) => void {
    return (...args) => {
        if (enabledPatterns().some((p) => p.test(namespace))) console.debug(`[${namespace}]`, ...args);
    };
}

/** Forget the cached setting (tests). */
export function _resetDebugLogForTests(): void {
    patterns = undefined;
}
