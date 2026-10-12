// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * This window's theme and opacity: its own value when it has one (the
 * window object's meta), otherwise the channel default from Settings →
 * Appearance. See docs/specs/SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12.md.
 *
 * Read the theme through `currentWindowTheme()`, never the `window:theme`
 * setting directly: the setting is only the default.
 */

import { createMemo, createRoot, createSignal } from "solid-js";

import { settingsAtom } from "./config-signals";
import { getApi } from "./app-api";
import { makeORef } from "./mos";
import { ObjectService, WindowService } from "./services";
import { muxWindow, windowId } from "./window-identity";

type Meta = Record<string, unknown> | null | undefined;

/** A window's theme: its own, else the channel default, else "default". */
export function themeFor(windowMeta: Meta, settings: Meta): string {
    const own = windowMeta?.["window:theme"];
    if (typeof own === "string" && own) return own;
    const dflt = settings?.["window:theme"];
    return typeof dflt === "string" && dflt ? dflt : "default";
}

/**
 * A window's opacity: its own, else the channel default. The default is what
 * every window showed before windows had their own: `window:opacity` (0.8
 * when unset) when the channel is see-through, otherwise fully opaque.
 */
export function opacityFor(windowMeta: Meta, settings: Meta): number {
    const own = windowMeta?.["window:opacity"];
    if (typeof own === "number") return own;
    const seeThrough = !!(settings?.["window:transparent"] || settings?.["window:blur"]);
    if (!seeThrough) return 1.0;
    const dflt = settings?.["window:opacity"];
    return typeof dflt === "number" ? Math.max(0, Math.min(1, dflt)) : 0.8;
}

// Built on first use, not at import, so importing this module reads nothing.
let memos: { theme: () => string; opacity: () => number } | undefined;
function getMemos(): { theme: () => string; opacity: () => number } {
    memos ??= createRoot(() => ({
        theme: createMemo(() => themeFor(muxWindow()?.meta, settingsAtom())),
        opacity: createMemo(() => opacityFor(muxWindow()?.meta, settingsAtom())),
    }));
    return memos;
}

/** The theme this window resolves to. Reactive. */
export const currentWindowTheme = (): string => getMemos().theme();

/** The opacity this window resolves to (its committed value, not a drag in progress). Reactive. */
export const currentWindowOpacity = (): number => getMemos().opacity();

// The theme whose `data-theme` is on this document now. Set by
// AppSettingsUpdater after it changes the attribute, so code that reads the
// theme's CSS variables (the terminal palette) recomputes after the change,
// not before it.
const [appliedTheme, setAppliedTheme] = createSignal<string>("default");
export const appliedWindowTheme = appliedTheme;
export const markWindowThemeApplied = setAppliedTheme;

/** Give this window its own theme. Other windows don't change. */
export function setCurrentWindowTheme(themeId: string): Promise<void> {
    const id = windowId();
    if (!id) return Promise.resolve();
    return ObjectService.UpdateObjectMeta(makeORef("window", id), { "window:theme": themeId } as MetaType);
}

// This renderer's host window label, fetched once on first use.
const [ownLabel, setOwnLabel] = createSignal<string | null>(null);
let labelRequested = false;
/** This window's host label, or null until the host has answered. Reactive. */
export function currentWindowLabel(): string | null {
    if (!labelRequested) {
        labelRequested = true;
        getApi()
            .getWindowLabel()
            .then((l) => setOwnLabel(l))
            .catch((e: unknown) => console.error("[window-appearance] getWindowLabel failed:", e));
    }
    return ownLabel();
}

/**
 * Ask srv to give the window with host label `newLabel` this window's theme
 * and opacity. Call it with the label `openNewWindow` or a tear-off returns;
 * srv applies it whether the new window's row exists yet or not.
 */
export function inheritAppearanceInto(newLabel: string | null | undefined): void {
    const source = windowId();
    if (!source || !newLabel) return;
    WindowService.InheritWindowAppearance(source, newLabel).catch((e: unknown) =>
        console.error("[window-appearance] InheritWindowAppearance failed:", e)
    );
}

/** Open a new window that starts with this window's theme and opacity. */
export async function openNewWindowFromHere(): Promise<string> {
    const label = await getApi().openNewWindow();
    inheritAppearanceInto(label);
    return label;
}
