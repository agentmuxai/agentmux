// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, onCleanup } from "solid-js";
import { isLightThemeActive } from "../block/pane-identity";
import { currentWindowTheme } from "../store/window-appearance";
import { resolveBackdrop, type ResolvedBackdrop } from "./meta";
import { registerPaneScope, releasePaneScope, scopeVars } from "./scope";

/** The app theme backdrops step aside for: it has its own fixed contrast (spec §8, §12). */
export const BACKDROP_SUPPRESSED_THEME = "high-contrast";

/** Whether the system's forced-colors mode is on, which overrides every colour a page sets (spec §12). */
function forcedColors(): boolean {
    return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(forced-colors: active)").matches;
}

/**
 * A pane's backdrop, from its meta, kept registered as its scope (`registerPaneScope`) for as long as the pane
 * lives. It follows the app theme: a switch re-derives the same backdrop for the other side. Returns the
 * resolved backdrop (for the pane's `data-backdrop`), or undefined when the pane has none to show: no record, or
 * `high-contrast` or forced colours are on, when the pane keeps the app's tokens.
 *
 * Reads only what it is given and writes only the registry: a pane with no backdrop and no plain-colour `bg`
 * registers nothing and draws exactly as before.
 */
export function usePaneBackdrop(blockId: string, meta: () => { backdrop?: unknown; bg?: unknown } | undefined): () => ResolvedBackdrop | undefined {
    const resolved = createMemo(() => {
        if (currentWindowTheme() === BACKDROP_SUPPRESSED_THEME || forcedColors()) return undefined;
        return resolveBackdrop(meta(), isLightThemeActive() ? "light" : "dark");
    });
    createEffect(() => {
        const r = resolved();
        registerPaneScope(blockId, r ? scopeVars(r.derived) : undefined);
    });
    onCleanup(() => releasePaneScope(blockId));
    return resolved;
}
