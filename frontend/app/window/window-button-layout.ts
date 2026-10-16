// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which window buttons the Linux title bar shows, and on which side: the
// desktop's own setting, GNOME's `org.gnome.desktop.wm.preferences
// button-layout`, read by the host once at startup.
// docs/specs/SPEC_LINUX_WINDOW_CONTROLS_GNOME_STYLE_2026_10_16.md §3.2

import { getApi } from "@/app/store/app-api";
import { createSignal } from "solid-js";

export type WindowButton = "minimize" | "maximize" | "close";

export interface WindowButtonLayout {
    left: WindowButton[];
    right: WindowButton[];
}

/** Ubuntu's default, and today's set and order: all three, on the right. */
export const DEFAULT_WINDOW_BUTTON_LAYOUT: WindowButtonLayout = { left: [], right: ["minimize", "maximize", "close"] };

const BUTTONS: readonly string[] = ["minimize", "maximize", "close"];

/**
 * Parse a GNOME button layout such as `"appmenu:minimize,maximize,close"`:
 * names left of the colon go on the left, the rest on the right, in order.
 * Names the app doesn't draw (`appmenu`, `icon`, `menu`, `spacer`, unknown
 * ones) and repeats are dropped. Close is always kept: a layout without it
 * gets it on the right, because a window with no close button is a trap.
 * Null, empty or unparsable input gives the default.
 */
export function parseWindowButtonLayout(raw: string | null | undefined): WindowButtonLayout {
    if (raw == null || !raw.trim()) return DEFAULT_WINDOW_BUTTON_LAYOUT;
    const [leftPart, rightPart = ""] = raw.split(":", 2);
    const seen = new Set<string>();
    const side = (part: string): WindowButton[] =>
        part
            .split(",")
            .map((n) => n.trim())
            .filter((n) => BUTTONS.includes(n) && !seen.has(n) && seen.add(n))
            .map((n) => n as WindowButton);
    const layout = { left: side(leftPart), right: side(rightPart) };
    if (!seen.has("close")) layout.right.push("close");
    return layout;
}

const [layout, setLayout] = createSignal<WindowButtonLayout>(DEFAULT_WINDOW_BUTTON_LAYOUT);

/** The window buttons to draw (reactive). The default until the host answers. */
export const windowButtonLayout = layout;

/** Ask the host once, at startup (initGlobal, Linux only). Safe on any host. */
export function loadWindowButtonLayout(): Promise<void> {
    let ask: Promise<string | null> | undefined;
    try {
        ask = getApi().getWindowButtonLayout?.();
    } catch {
        return Promise.resolve();
    }
    if (!ask) return Promise.resolve();
    return ask.then(
        (raw) => {
            setLayout(parseWindowButtonLayout(raw));
        },
        () => {}
    );
}

/** Back to the default, so tests start clean. */
export function resetWindowButtonLayoutForTest(): void {
    setLayout(DEFAULT_WINDOW_BUTTON_LAYOUT);
}
