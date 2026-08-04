// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Per-region scroll + inert lock (SPEC_UNIFIED_MODAL_SYSTEM §5) ─────────
// Inert only the *lock region's* element children that aren't a modal
// root, and scroll-lock only that region. Window-scope locks the
// document body (every other element goes inert); tab-scope locks
// just that tab's content; pane-scope locks just that pane.
//
// Reference-counted, keyed per lock-region element (a WeakMap keyed by
// the region node) — so stacked modals sharing a region release the lock
// only when the last one closes, while modals in disjoint regions never
// interfere.

interface RegionLockState {
    openCount: number;
    previousOverflow: string;
    inertSiblings: HTMLElement[];
}

const regionLocks = new WeakMap<HTMLElement, RegionLockState>();

const supportsInert = typeof HTMLElement !== "undefined" && "inert" in HTMLElement.prototype;

/**
 * Acquire the scroll + inert lock for `region`. The first modal in a
 * region performs the real lock; later modals just bump the count.
 *
 * Inert is applied to `region`'s direct element children that are not a
 * `modal-root` and not already inert. For window scope `region` is the
 * document body. For tab/pane scope only that region's children are
 * inerted, leaving the rest of the page live.
 */
export function acquireRegionLock(region: HTMLElement): void {
    const existing = regionLocks.get(region);
    if (existing) {
        existing.openCount++;
        return;
    }
    const state: RegionLockState = {
        openCount: 1,
        previousOverflow: region.style.overflow,
        inertSiblings: [],
    };
    region.style.overflow = "hidden";
    if (supportsInert) {
        for (const child of Array.from(region.children) as HTMLElement[]) {
            if (!child.classList.contains("modal-root") && !child.hasAttribute("inert")) {
                child.setAttribute("inert", "");
                state.inertSiblings.push(child);
            }
        }
    }
    regionLocks.set(region, state);
}

/**
 * Release the lock for `region`. When the last modal in the region
 * closes, scroll is restored and inert cleared. Lower modals sharing the
 * region keep the lock alive until they're gone too.
 */
export function releaseRegionLock(region: HTMLElement): void {
    const state = regionLocks.get(region);
    if (!state) return;
    state.openCount--;
    if (state.openCount > 0) return;
    for (const el of state.inertSiblings) el.removeAttribute("inert");
    region.style.overflow = state.previousOverflow;
    regionLocks.delete(region);
}
