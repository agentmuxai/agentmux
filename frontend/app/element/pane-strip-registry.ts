// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The strips a pane draws (its pane tab strip, and a document tab strip when
// its block is an Editor or Media view) register here under the pane's root
// element, and PaneChrome asks them all to reveal their selected tab on a
// focus or an interaction (docs/specs/SPEC_PANE_TAB_STRIP_REVEAL_SELECTED_2026_10_15.md
// §4). Keyed by DOM, not a Solid context: a view's content is built before
// PaneChrome renders and is not under its owner, so a context would never
// reach a document tab strip inside it.

export type StripRevealReason = "selected" | "visible" | "layout" | "focus" | "interaction";

export interface PaneStripHandle {
    /** Asks the strip to bring its selected tab into view; coalesced and cheap when there is nothing to do. */
    revealSelected(reason: StripRevealReason): void;
    /** Whether `node` is inside the strip: its own events never reveal it. */
    contains(node: Node | null): boolean;
}

/** PaneChrome's root; the same selector the strips use for the cross-pane drop zone. */
export const PANE_ROOT_SELECTOR = '[data-role="pane"]';

const registries = new WeakMap<Element, Set<PaneStripHandle>>();

/** A pane takes the focus from a press in it: a focus this soon after an interaction is that interaction's. */
const FOCUS_FOLLOWS_INTERACTION_MS = 500;

const lastInteraction = new WeakMap<Element, { origin: Node; at: number }>();

/** Registers `handle` with the pane around `strip`. Returns the unregister; a no-op for a strip outside any pane. */
export function registerPaneStrip(strip: Element, handle: PaneStripHandle): () => void {
    const root = strip.closest(PANE_ROOT_SELECTOR);
    if (!root) return () => {};
    let set = registries.get(root);
    if (!set) registries.set(root, (set = new Set()));
    set.add(handle);
    return () => {
        set.delete(handle);
    };
}

/**
 * Asks every strip of the pane to reveal, except those holding `origin`. A focus with no origin of its own takes
 * the origin of the interaction it follows, so a press on a strip's own box that focuses the pane does not
 * scroll that box from under the pointer.
 */
export function revealPaneStrips(root: Element, reason: StripRevealReason, origin: Node | null = null): void {
    const set = registries.get(root);
    if (!set) return;
    if (!origin && reason === "focus") {
        const last = lastInteraction.get(root);
        if (last && performance.now() - last.at < FOCUS_FOLLOWS_INTERACTION_MS) origin = last.origin;
    }
    for (const handle of set) if (!origin || !handle.contains(origin)) handle.revealSelected(reason);
}

const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "CapsLock", "AltGraph", "OS"]);

const INTERACTIONS = ["pointerdown", "keydown", "beforeinput", "input", "paste", "compositionstart"] as const;

/**
 * Listens on the pane's root, capture phase, for a click or any input into the
 * pane and asks its strips to reveal. Reads no layout: each strip decides, a
 * frame later, whether it has anything to do. Returns the removal.
 */
export function installPaneInteractionReveal(root: HTMLElement): () => void {
    const onEvent = (e: Event) => {
        if (e.type === "keydown" && MODIFIER_KEYS.has((e as KeyboardEvent).key)) return;
        const origin = e.target instanceof Node ? e.target : null;
        if (origin) lastInteraction.set(root, { origin, at: performance.now() });
        revealPaneStrips(root, "interaction", origin);
    };
    for (const type of INTERACTIONS) root.addEventListener(type, onEvent, { capture: true, passive: true });
    return () => {
        for (const type of INTERACTIONS) root.removeEventListener(type, onEvent, { capture: true });
    };
}
