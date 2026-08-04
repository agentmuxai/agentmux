// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Cancel nudge (spec §9) ──────────────────────────────────────────────────
// When `closeOnBackdropClick` is false, a backdrop click must NOT close
// the modal — instead it nudges the panel's primary dismiss affordance.
// We find `[data-modal-dismiss]` inside the panel, add the
// `modal-dismiss--nudge` class, and remove it on `animationend` so a
// later click re-triggers the keyframe. No-op when the panel has no
// dismiss control. The reduced-motion CSS variant is still a (brief,
// non-moving) animation so `animationend` reliably fires.

const NUDGE_CLASS = "modal-dismiss--nudge";

export function nudgeDismissControl(panel: HTMLElement | undefined): void {
    if (!panel) return;
    const target = panel.querySelector<HTMLElement>("[data-modal-dismiss]");
    if (!target) return;
    // Restart the animation if a previous nudge is still mid-flight.
    target.classList.remove(NUDGE_CLASS);
    // Force a reflow so removing + re-adding the class restarts the keyframe.
    void target.offsetWidth;
    target.classList.add(NUDGE_CLASS);
    const onEnd = (): void => {
        target.classList.remove(NUDGE_CLASS);
        target.removeEventListener("animationend", onEnd);
        target.removeEventListener("animationcancel", onEnd);
    };
    target.addEventListener("animationend", onEnd);
    target.addEventListener("animationcancel", onEnd);
}
