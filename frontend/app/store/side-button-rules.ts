// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a mouse side-button press means, as pure functions (side-buttons.ts
// wires them to the window and the host).
// SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §4.2.

export const SIDE_BUTTON_BACK = 3;
export const SIDE_BUTTON_FORWARD = 4;
/** A second press of the same button within this is the same press, reported twice. */
export const DEBOUNCE_MS = 150;

export interface SideButtonInput {
    button: number;
    /** The buttons still held (bit 1 = left). */
    buttons: number;
    ctrlKey: boolean;
    shiftKey: boolean;
    altKey: boolean;
    metaKey: boolean;
}

export interface SideButtonContext {
    enabled: boolean;
    modalOpen: boolean;
    /** A context menu (the pane menu, the "+" picker, a row's menu) is open. */
    menuOpen: boolean;
}

export interface SideButtonDecision {
    /** Keep the page and the terminal from seeing it, and Chromium from navigating. */
    consume: boolean;
    /** Back (-1) or Forward (+1) in the selection history; null for nothing. */
    act: -1 | 1 | null;
}

export function decideSideButton(e: SideButtonInput, ctx: SideButtonContext): SideButtonDecision {
    const dir = e.button === SIDE_BUTTON_BACK ? -1 : e.button === SIDE_BUTTON_FORWARD ? 1 : null;
    if (dir == null || !ctx.enabled) return { consume: false, act: null };
    const plain = !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
    // Left held is a drag-select or a pane drag: the press isn't a navigation.
    const idle = (e.buttons & 1) === 0;
    return { consume: true, act: plain && idle && !ctx.modalOpen && !ctx.menuOpen ? dir : null };
}

/** What the host reports of a press the page never saw as its own. */
export interface HostSideButtonPress {
    /** Ctrl was down: over a browser pane that is the page's own history, done in the host. */
    ctrl: boolean;
    /** Shift, Alt, a Windows key or the left button was down: the page listener declines these. */
    blocked?: boolean;
}

/** Whether a host-reported press moves the selection history: the same rules as the page listener's. */
export function hostPressActs(p: HostSideButtonPress, ctx: SideButtonContext): boolean {
    return ctx.enabled && !ctx.modalOpen && !ctx.menuOpen && !p.ctrl && !p.blocked;
}

/** True when this press repeats `last` closely enough to be the same press. */
export function isRepeat(dir: -1 | 1, now: number, last: { dir: -1 | 1; at: number } | null): boolean {
    return last != null && last.dir === dir && now - last.at < DEBOUNCE_MS;
}
