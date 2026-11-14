// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A modal's backdrop fades out when it closes.
//
// The backdrop dims and blurs everything behind its dialog and fades in with it. Closing used to
// remove it in one frame, so whatever sat behind the dialog jumped from dim and blurred to bright
// and sharp at once. When that was a pane's conversation and the agent restarted in the same moment
// (a sign-in that moves the agent to the new account), it read as the conversation flashing.
//
// The dialog itself is gone as before. This leaves a stand-in with only a backdrop, placed like the
// modal's root, for one quick fade, so the modal's own mount and unmount are untouched. It is not
// a `.modal-root`, so nothing that looks for open dialogs finds it.

import { DURATION_QUICK_MS } from "@/app/motion/tokens";
import { scaledMs, shouldReduceMotion } from "@/app/motion/motion";

/** How much longer than the fade the copy is kept if its `animationend` never comes. */
const SLACK_MS = 60;

/**
 * Fade out a closed modal's backdrop. `root` is the modal's `.modal-root` (still in the page) and
 * `mount` the element it was drawn into. Does nothing when this window's motion policy reduces
 * transitions or the duration scales to nothing.
 */
export function fadeOutBackdrop(root: HTMLElement | null | undefined, mount: HTMLElement | null | undefined): void {
    if (!root || !mount || shouldReduceMotion("transition")) return;
    const ms = scaledMs(DURATION_QUICK_MS);
    if (ms <= 0) return;
    const copy = root.ownerDocument.createElement("div");
    copy.className = "modal-leaving";
    copy.setAttribute("aria-hidden", "true");
    for (const name of ["data-scope", "data-placement"]) {
        const value = root.getAttribute(name);
        if (value !== null) copy.setAttribute(name, value);
    }
    const backdrop = root.ownerDocument.createElement("div");
    backdrop.className = "modal-backdrop";
    copy.append(backdrop);
    mount.append(copy);
    const remove = () => copy.remove();
    backdrop.addEventListener("animationend", remove, { once: true });
    setTimeout(remove, ms + SLACK_MS);
}
