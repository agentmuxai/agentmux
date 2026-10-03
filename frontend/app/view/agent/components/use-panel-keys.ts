// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, onCleanup } from "solid-js";
import { eventBelongsToPaneOf, isEditableTarget } from "@/util/focusutil";

export interface PanelKeyContext {
    /** The key came from inside the panel itself. */
    inPanel: boolean;
    /** The key came from somewhere the user types (composer, search, a field). */
    editable: boolean;
}

/**
 * The key plumbing the decision and question panels share, installed while
 * `active()` is true:
 * - a capture-phase `window` keydown listener, because the panel is
 *   `tabindex=-1` and never focused, so a listener on it would never fire
 *   (codex P1, PR #556);
 * - scoped to this panel's pane, so a prompt in pane A ignores keys typed in
 *   pane B (`eventBelongsToPaneOf`);
 * - `onKey` gets whether the key came from inside the panel and from an
 *   editable target, which every Enter/Escape rule depends on.
 *
 * `onFocusIn`, if given, gets `focusin` events that land inside the panel
 * (Tab into it fires keydown on the element focus is leaving, codex P2, PR #2787).
 */
export function usePanelKeys(
    root: () => HTMLElement | undefined,
    active: () => boolean,
    onKey: (e: KeyboardEvent, ctx: PanelKeyContext) => void,
    onFocusIn?: (e: FocusEvent) => void
): void {
    const contains = (target: EventTarget | null) => {
        const el = root();
        return !!el && !!target && el.contains(target as Node);
    };
    createEffect(() => {
        if (!active()) return;
        const handleKey = (e: KeyboardEvent) => {
            if (!eventBelongsToPaneOf(e, root())) return;
            onKey(e, { inPanel: contains(e.target), editable: isEditableTarget(e.target) });
        };
        const handleFocusIn = (e: FocusEvent) => {
            if (contains(e.target)) onFocusIn?.(e);
        };
        window.addEventListener("keydown", handleKey, true);
        // `focusin` bubbles to `window`; no capture needed.
        if (onFocusIn) window.addEventListener("focusin", handleFocusIn);
        onCleanup(() => {
            window.removeEventListener("keydown", handleKey, true);
            if (onFocusIn) window.removeEventListener("focusin", handleFocusIn);
        });
    });
}
