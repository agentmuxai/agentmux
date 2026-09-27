// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Scroll hand-off from a capped preview box to the agent pane
 * (SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md Phase 2; shared by every
 * capped transcript box per SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §2).
 *
 * Native scroll chaining latches a wheel gesture to the inner box, so reaching
 * its edge mid-gesture either dead-ends or jerks the pane (found live; Phase 1,
 * CSS only, was not enough). So the box sets `overscroll-behavior: contain` —
 * which blocks the browser's own relay unconditionally — and this listener
 * does the relay by hand: once the box can't consume more scroll in the
 * wheel's direction, the same delta goes to `.agent-document` directly, on the
 * very next tick. Bubbling doesn't help: `contain` isn't an event-propagation
 * setting.
 *
 * Returns the cleanup. The box must carry `overscroll-behavior: contain`.
 */
export function attachScrollHandoff(el: HTMLElement): () => void {
    const onWheel = (e: WheelEvent): void => {
        // Ctrl+wheel is a ZOOM gesture (the pane zoom in app.tsx), never a
        // scroll: forwarding it at a boundary would zoom AND scroll at once.
        if (e.ctrlKey) return;
        const atTop = el.scrollTop <= 0;
        // 1 px slack: fractional layout at non-100% zoom leaves a true bottom
        // a hair short of scrollHeight.
        const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
        const scrollingUp = e.deltaY < 0;
        const scrollingDown = e.deltaY > 0;
        if (!((atTop && scrollingUp) || (atBottom && scrollingDown))) return;
        const pane = el.closest<HTMLElement>(".agent-document");
        if (!pane) return;
        e.preventDefault();
        pane.scrollTop += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
}
