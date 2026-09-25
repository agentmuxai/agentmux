// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Composer keyboard focus for the agent pane.
 *
 * Two entry points:
 *
 *   - `focusComposer(ta)` — what `AgentViewModel.giveFocus()` calls (via the
 *     handle AgentFooter registers), so every generic "focus this pane" path
 *     — keyboard pane navigation, refocusNode, window re-activation — lands
 *     in the composer instead of the block's hidden dummy input.
 *
 *   - `requestComposerFocus(blockId)` + `focusComposerWhenReady(...)` — a
 *     one-shot request from the launch path. Picking an agent in My Agents
 *     leaves DOM focus on the clicked picker button, which is removed ~200ms
 *     later, dropping focus to <body>. AgentFooter consumes the request when
 *     it mounts and moves focus into the composer as soon as it can: at once
 *     for a picker click, or — when launched from the launch modal, which
 *     inerts the pane content until it closes — once the modal is gone.
 *
 * Spec: SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3.
 */

/** A request older than this is ignored: it was never consumed by a mount
 *  (e.g. relaunch into a block whose footer was already mounted), and
 *  honoring it on some unrelated later mount would steal focus. */
const REQUEST_TTL_MS = 10_000;

/** How long a consumed request keeps trying to land focus. Covers the
 *  launch modal staying open (content inert) for the rest of the launch
 *  RPC chain after the agent's meta is written. */
export const LAUNCH_FOCUS_WINDOW_MS = 5_000;
const LAUNCH_FOCUS_POLL_MS = 50;

const pendingRequests = new Map<string, number>();

/** Ask the composer of `blockId` to take focus when it next mounts. */
export function requestComposerFocus(blockId: string): void {
    pendingRequests.set(blockId, Date.now());
}

export function cancelComposerFocusRequest(blockId: string): void {
    pendingRequests.delete(blockId);
}

/** Consume a pending request for `blockId`. True if one was pending and fresh. */
export function takeComposerFocusRequest(blockId: string): boolean {
    const at = pendingRequests.get(blockId);
    pendingRequests.delete(blockId);
    return at != null && Date.now() - at <= REQUEST_TTL_MS;
}

function blockIdOf(el: Element | null): string | null {
    return el?.closest("[data-blockid]")?.getAttribute("data-blockid") ?? null;
}

function isDummyFocus(el: Element): boolean {
    return el.id.endsWith("-dummy-focus");
}

function isTextEntry(el: Element): boolean {
    if (isDummyFocus(el)) return false;
    const tag = el.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || (el as HTMLElement).isContentEditable;
}

/**
 * Focus the composer, unless doing so would clobber something:
 *   - it sits under an `inert` ancestor (a pane-scope modal is open) → false;
 *   - the user has a non-collapsed text selection in this block (a click
 *     that ends a drag-select would otherwise wipe it) → false, so the
 *     caller's fallback runs exactly as it did before giveFocus existed;
 *   - another text entry in this block already has focus (login panel,
 *     decision panel) → true without moving focus: it's handled.
 * Caret goes to the end so a restored draft is appended to, not prepended.
 */
export function focusComposer(ta: HTMLTextAreaElement): boolean {
    if (ta.closest("[inert]")) return false;
    const block = ta.closest("[data-blockid]");
    const sel = document.getSelection();
    if (sel && !sel.isCollapsed && sel.anchorNode && block?.contains(sel.anchorNode)) return false;
    const active = document.activeElement;
    if (active && active !== ta && block?.contains(active) && isTextEntry(active)) return true;
    // The composer's own scroller handles its content; letting the browser
    // scroll ancestors to reveal it shifted whole tabs
    // (REPORT_TAB_PANES_OFFSET_HALF_WINDOW_2026_09_24.md).
    ta.focus({ preventScroll: true });
    const end = ta.value.length;
    ta.setSelectionRange(end, end);
    return document.activeElement === ta;
}

type Readiness = "focus" | "wait" | "stop";

/** Whether the launch-path request should focus now, keep waiting, or give up. */
function launchFocusReadiness(ta: HTMLTextAreaElement, blockId: string): Readiness {
    const active = document.activeElement;
    if (active === ta) return "stop"; // already there
    if (active && active !== document.body) {
        // A modal (e.g. the launch modal while it finishes submitting) —
        // it will close; wait for it.
        if (active.closest(".modal-root")) return "wait";
        const owner = blockIdOf(active);
        // The user moved on to another pane during the launch.
        if (owner != null && owner !== blockId) return "stop";
        // Something that takes typing already has focus — a panel in this
        // pane, or an input elsewhere in the window. Leave it alone.
        if (isTextEntry(active)) return "stop";
    }
    // Focus is on <body>, or on a non-text element of this pane (the
    // clicked My Agents button that is about to be removed).
    return ta.closest("[inert]") ? "wait" : "focus";
}

/**
 * Move focus into `ta` as soon as that is possible and appropriate, for up
 * to `LAUNCH_FOCUS_WINDOW_MS`. Returns a cancel function (call on unmount).
 */
export function focusComposerWhenReady(ta: HTMLTextAreaElement, blockId: string): () => void {
    const deadline = Date.now() + LAUNCH_FOCUS_WINDOW_MS;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const attempt = (): void => {
        timer = null;
        const readiness = launchFocusReadiness(ta, blockId);
        if (readiness === "stop") return;
        if (readiness === "focus" && focusComposer(ta) && document.activeElement === ta) return;
        if (Date.now() < deadline) timer = setTimeout(attempt, LAUNCH_FOCUS_POLL_MS);
    };
    attempt();
    return () => {
        if (timer != null) clearTimeout(timer);
        timer = null;
    };
}
