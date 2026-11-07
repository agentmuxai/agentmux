// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What reaches the agent pane's find bar from outside the pane's view: the magnifier in the pane's header and the
 * command palette (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.1). The bar's state lives in the pane
 * (`useInSessionSearch`); a pane registers what outside code may do with it, and the header button and the
 * palette read it by block id. One open path: the magnifier, Ctrl+F and the palette all end in the same `open`.
 */

import { isMacOS } from "@/util/platformutil";
import type { SignalAtom } from "@/util/util";
import { createSignal, type Accessor } from "solid-js";

export interface SearchControls {
    /** The bar is open. */
    visible: Accessor<boolean>;
    /** Open the bar and focus its field; with the bar already open, select the field's text. */
    open: () => void;
    /** The magnifier: open the bar, or close it. */
    toggle: () => void;
}

interface Registered {
    controls: SearchControls;
    button: ToggleIconButtonDecl;
}

const [registry, setRegistry] = createSignal<Record<string, Registered>>({});

/** The magnifier's declaration: pressed while the bar is open; a click asks the pane to toggle it. */
function magnifier(controls: SearchControls): ToggleIconButtonDecl {
    // A fake SignalAtom, as the always-on-top tack uses: the read tracks the bar, the write toggles it.
    const active = (() => controls.visible()) as SignalAtom<boolean>;
    active._set = () => controls.toggle();
    return {
        elemtype: "toggleiconbutton",
        icon: "magnifying-glass",
        title: `Find in conversation (${isMacOS() ? "Cmd" : "Ctrl"}+F)`,
        className: "agent-search-toggle",
        active,
    };
}

/** A pane offers its find bar to its header and the palette; the returned function takes it back. */
export function registerSearchControls(blockId: string, controls: SearchControls): () => void {
    const entry: Registered = { controls, button: magnifier(controls) };
    setRegistry((prev) => ({ ...prev, [blockId]: entry }));
    return () =>
        setRegistry((prev) => {
            if (prev[blockId] !== entry) return prev;
            const { [blockId]: _gone, ...rest } = prev;
            return rest;
        });
}

/** The find bar of a pane, for what acts on it from outside (the palette). */
export const searchControlsFor = (blockId: string): SearchControls | undefined => registry()[blockId]?.controls;

/** The header buttons the find bar has for a pane: its magnifier, first among the pane's end buttons. Reactive. */
export function searchHeaderButtons(blockId: string): ToggleIconButtonDecl[] {
    const entry = registry()[blockId];
    return entry ? [entry.button] : [];
}

/** The pane's frame, to find the bar's parts inside it. */
const frame = (blockId: string): ParentNode | null => document.querySelector(`[data-blockid="${blockId}"]`);

/** Put the caret in the bar's field, with its text selected. */
export function focusSearchField(blockId: string): void {
    const field = frame(blockId)?.querySelector<HTMLInputElement>(".agent-search-input");
    field?.focus();
    field?.select();
}

/**
 * The bar closed: give focus back to the magnifier, so it is never lost to the page. Only when focus was in the bar
 * (or nowhere, because the bar that held it is gone): a close that came from elsewhere leaves focus where it is.
 */
export function restoreFocusToMagnifier(blockId: string): void {
    const at = document.activeElement;
    if (at && at !== document.body && !at.closest(".agent-search-bar")) return;
    frame(blockId)?.querySelector<HTMLElement>(".agent-search-toggle")?.focus();
}
