// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the host's keyboard hook saw Escape during the drag in progress.
 *
 * The page doesn't see the key while the OS runs a drag (native-cancel.ts), so on Windows the host reports it
 * (`tabdrag:escape-pressed`). That is the one signal that doesn't depend on a later read of the mouse button,
 * which a busy host answers after the user has let go. Reset when a drag begins (drag-session.ts).
 */

let seen = false;

/** The host reported Escape. */
export function noteHostEscape(): void {
    seen = true;
}

/** Escape was reported since the drag began. */
export function hostEscapeSeen(): boolean {
    return seen;
}

/** A new drag begins: forget the last one's Escape. */
export function resetHostEscape(): void {
    seen = false;
}
