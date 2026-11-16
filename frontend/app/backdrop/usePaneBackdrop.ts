// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { isLightThemeActive } from "../block/pane-identity";
import { currentWindowTheme } from "../store/window-appearance";
import { themeAnsi } from "./ansi";
import { resolveBackdrop, type ResolvedBackdrop } from "./meta";
import { registerPaneScope, releasePaneScope, scopeVars } from "./scope";

/** The app theme backdrops step aside for: it has its own fixed contrast (spec §8, §12). */
export const BACKDROP_SUPPRESSED_THEME = "high-contrast";

const FORCED_COLORS = "(forced-colors: active)";

/** Whether the system's forced-colors mode is on, which overrides every colour a page sets (spec §12). Follows the system while the owner lives. */
function createForcedColors(): Accessor<boolean> {
    const list = typeof window !== "undefined" && typeof window.matchMedia === "function" ? window.matchMedia(FORCED_COLORS) : undefined;
    const [on, setOn] = createSignal(!!list?.matches);
    if (list && typeof list.addEventListener === "function") {
        const onChange = (e: MediaQueryListEvent) => setOn(e.matches);
        list.addEventListener("change", onChange);
        onCleanup(() => list.removeEventListener("change", onChange));
    }
    return on;
}

/**
 * Whether backdrops are suppressed now: the `high-contrast` app theme, or forced colours (spec §8, §12). One place for
 * it, read by the hook that keeps a pane's scope and by the picker's preview, so neither can draw a backdrop the other
 * has stepped aside for.
 */
export function useBackdropSuppressed(): Accessor<boolean> {
    const forced = createForcedColors();
    return () => currentWindowTheme() === BACKDROP_SUPPRESSED_THEME || forced();
}

/**
 * The backdrop a pane's meta resolves to under the app theme now, or undefined when it has none to show: no record, or
 * backdrops are suppressed. The same answer for the pane's frame and for the picker.
 */
export function useResolvedBackdrop(
    meta: () => { backdrop?: unknown; bg?: unknown } | undefined,
    suppressed: Accessor<boolean> = useBackdropSuppressed()
): Accessor<ResolvedBackdrop | undefined> {
    return createMemo(() => (suppressed() ? undefined : resolveBackdrop(meta(), isLightThemeActive() ? "light" : "dark")));
}

/**
 * A pane's backdrop, from its meta, kept registered as its scope (`registerPaneScope`) for as long as the pane
 * lives. It follows the app theme: a switch re-derives the same backdrop for the other side. Returns the
 * resolved backdrop (for the pane's `data-backdrop`), or undefined when the pane has none to show: no record, or
 * `high-contrast` or forced colours are on, when the pane keeps the app's tokens.
 *
 * Reads only what it is given and writes only the registry: a pane with no backdrop and no plain-colour `bg`
 * registers nothing and draws exactly as before.
 *
 * Only the pane's live frame owns its registry entry (`owner`, the default). A second frame for the same block, a
 * drag preview, resolves the same backdrop for its own `data-backdrop` and reads the live scope, but never
 * registers or releases it: its unmounting must not take the live pane's scope with it.
 */
export function usePaneBackdrop(
    blockId: string,
    meta: () => { backdrop?: unknown; bg?: unknown } | undefined,
    { owner = true }: { owner?: boolean } = {}
): () => ResolvedBackdrop | undefined {
    const resolved = useResolvedBackdrop(meta);
    if (owner) {
        createEffect(() => {
            const r = resolved();
            // The theme's terminal palette is read only for a pane that has a backdrop, once per theme change.
            registerPaneScope(blockId, r ? scopeVars(r.derived, themeAnsi()) : undefined);
        });
        onCleanup(() => releasePaneScope(blockId));
    }
    return resolved;
}
