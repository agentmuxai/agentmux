// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/store/global";
import { isWindows } from "@/util/platformutil";
import { fireAndForget } from "@/util/util";
import { createRedockArming } from "./redock-arming";

/**
 * Subscribe to the host's `floating-redock:hover-state` event so this
 * window paints a drop-slot preview when a floater is being dragged
 * over it. The host computes the target window via the same Z-order
 * walk `resolve_window_at_cursor` uses (with the dragged floater
 * excluded) and emits this event on every mousemove update; payload
 * includes the cursor position in physical screen px.
 *
 * Each window's renderer runs this in its own JS context. When the
 * event targets our window:
 *   1. Convert cursor screen-physical-px → client CSS px (DPR +
 *      `window.screenX/Y`, same pattern as `tabbar.tsx`).
 *   2. `document.elementFromPoint(clientX, clientY)` → walk to the
 *      nearest `[data-blockid]` ancestor.
 *   3. Run `determineDropDirection` (from `@/layout/lib/utils`) on
 *      the cursor's position within the leaf — same helper as
 *      within-window pane drag — to classify the drop as
 *      Center / Top / Right / Bottom / Left (and Outer variants).
 *   4. Render a singleton overlay div positioned over the
 *      half/quadrant/full-leaf that maps to that direction (e.g. Top
 *      → top half of leaf; Center → full leaf; OuterRight → right
 *      fifth of leaf).
 *
 * The block STILL lands as a sibling in the target tab's layout for
 * MVP (backend ignores the direction). Phase 4b will wire the
 * direction through `RedockFloatingPane` so the block lands in the
 * exact slot the user previewed.
 */
export function installFloatingRedockHoverListener(): void {
    const myLabel =
        new URLSearchParams(window.location.search).get("windowLabel") || "main";
    let placeholderEl: HTMLDivElement | null = null;

    const ensurePlaceholder = (): HTMLDivElement => {
        if (!placeholderEl) {
            placeholderEl = document.createElement("div");
            placeholderEl.className = "floating-redock-drop-placeholder";
            document.body.appendChild(placeholderEl);
        }
        return placeholderEl;
    };
    // Whether the host holds a drop target for this window: only while the
    // drag is armed (see the two stages below).
    let targetStored = false;
    const forgetTarget = () => {
        // Nothing stored, nothing to clear: this runs on every preview
        // sample, and an IPC per heartbeat to clear state that was never set
        // would be pure waste.
        if (!targetStored) return;
        targetStored = false;
        // Phase 4b — clear the stored ghost state for this window so a stale
        // direction cannot bleed into the next drop event.
        fireAndForget(async () => {
            await getApi().windows.setFloatingRedockTarget(myLabel, null, null);
        });
    };
    const clearPlaceholder = () => {
        if (placeholderEl) {
            placeholderEl.remove();
            placeholderEl = null;
        }
        forgetTarget();
    };

    // Map a DropDirection to a sub-rect (top, left, width, height in
    // client CSS px) within the leaf rect. Mirrors the visual the
    // within-window pane drag's `.placeholder` provides:
    //   Center      → full leaf
    //   Top/Bottom  → half along that axis
    //   Left/Right  → half along that axis
    //   Outer*      → thin (1/5) band against that edge
    const rectForDirection = (leaf: DOMRect, dir: number): { top: number; left: number; width: number; height: number } => {
        // DropDirection enum: Top=0, Right=1, Bottom=2, Left=3,
        //   OuterTop=4, OuterRight=5, OuterBottom=6, OuterLeft=7,
        //   Center=8.
        switch (dir) {
            case 0: // Top
                return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height / 2 };
            case 1: // Right
                return { top: leaf.top, left: leaf.left + leaf.width / 2, width: leaf.width / 2, height: leaf.height };
            case 2: // Bottom
                return { top: leaf.top + leaf.height / 2, left: leaf.left, width: leaf.width, height: leaf.height / 2 };
            case 3: // Left
                return { top: leaf.top, left: leaf.left, width: leaf.width / 2, height: leaf.height };
            case 4: // OuterTop
                return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height / 5 };
            case 5: // OuterRight
                return { top: leaf.top, left: leaf.left + (4 * leaf.width) / 5, width: leaf.width / 5, height: leaf.height };
            case 6: // OuterBottom
                return { top: leaf.top + (4 * leaf.height) / 5, left: leaf.left, width: leaf.width, height: leaf.height / 5 };
            case 7: // OuterLeft
                return { top: leaf.top, left: leaf.left, width: leaf.width / 5, height: leaf.height };
            case 8: // Center
            default:
                return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height };
        }
    };

    // Dwell gate for the redock ghost.
    //
    // Both sides run the SAME arming state machine over the SAME event stream,
    // so the ghost appears exactly when the floater considers the drag armed —
    // no per-platform clock to keep in sync, and no window where the ghost is
    // visible but a release would not dock (or vice versa). Previously this was
    // a second, independent dwell timer that had to be disabled outright on
    // non-Windows because the floater pre-gated its own IPC, which meant the
    // two sides were gating on different rules by construction.
    // SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md §5.1/§5.4.
    const ghostArming = createRedockArming();

    fireAndForget(async () => {
        const { determineDropDirection } = await import("@/layout/lib/utils");
        await getApi().listen<{
            target_label: string | null;
            source_label?: string;
            cursor_x?: number;
            cursor_y?: number;
        }>("floating-redock:hover-state", (payload) => {
            const newTarget = payload?.target_label ?? null;
            const cursorX = payload?.cursor_x;
            const cursorY = payload?.cursor_y;
            // The broadcast cursor is in the host's coordinate space: physical
            // px on Windows (divide by DPR to get CSS px), DIP on macOS/Linux
            // (already CSS px — no divide). Inverse of the sender's posScale()
            // in floating-pane-workspace.tsx. Without this the drop-zone
            // highlight lands wrong on a Retina display, and the velocity gate
            // would read 2× the real cursor speed.
            const invScale = isWindows() ? window.devicePixelRatio || 1 : 1;
            // A teardown broadcast (`clear_floating_redock_hover`) carries a
            // null target and no cursor. Feed it through as a null-target
            // sample so the module disarms, rather than special-casing it.
            const { armed } = ghostArming.sample({
                target: newTarget,
                x: typeof cursorX === "number" ? cursorX / invScale : undefined,
                y: typeof cursorY === "number" ? cursorY / invScale : undefined,
                t: performance.now(),
            });
            if (!payload || newTarget !== myLabel) {
                clearPlaceholder();
                return;
            }
            if (typeof cursorX !== "number" || typeof cursorY !== "number") {
                clearPlaceholder();
                return;
            }
            const clientX = cursorX / invScale - window.screenX;
            const clientY = cursorY / invScale - window.screenY;
            const el = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
            const leafEl = el?.closest("[data-blockid]") as HTMLElement | null;
            if (!leafEl) {
                clearPlaceholder();
                return;
            }
            const leafRect = leafEl.getBoundingClientRect();
            const dir = determineDropDirection(
                {
                    width: leafRect.width,
                    height: leafRect.height,
                    left: leafRect.left,
                    top: leafRect.top,
                },
                { x: clientX, y: clientY },
            );
            if (dir === undefined) {
                clearPlaceholder();
                return;
            }
            const slot = rectForDirection(leafRect, dir);
            const ph = ensurePlaceholder();
            ph.style.top = `${slot.top}px`;
            ph.style.left = `${slot.left}px`;
            ph.style.width = `${slot.width}px`;
            ph.style.height = `${slot.height}px`;

            // Two stages (SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md §10):
            // from the first sample over this window, a faint preview of where
            // the pane would land, so the drag answers at once; solid, and a
            // real drop target, only once the dwell arms it. A release on the
            // preview doesn't dock: the host holds no target for it, and the
            // floater runs the same arming over the same samples.
            ph.classList.toggle("floating-redock-drop-placeholder--preview", !armed);
            if (!armed) {
                forgetTarget();
                return;
            }

            // Phase 4b — store the computed direction and target block so
            // the floater can pass them to RedockFloatingPane at drop time.
            // Fire-and-forget: the set is best-effort and must not stall the
            // event handler (ghost rendering happens synchronously above).
            const targetBlockId = leafEl.dataset.blockid;
            if (targetBlockId) {
                targetStored = true;
                void getApi().windows.setFloatingRedockTarget(myLabel, targetBlockId, dir);
            }
        });
    });
}
