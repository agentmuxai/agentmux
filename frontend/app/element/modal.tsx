// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Modal — the unified, scope-based modal primitive.
 *
 * Stage 1 of SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21. This is `modal-v2`
 * evolved to add a first-class `scope` axis: what the modal *locks* —
 * the window, a tab, or a single pane. Mount point, backdrop extent,
 * `inert` boundary, scroll lock and the modal stack are all consequences
 * of that scope.
 *
 * `modal-v2.{tsx,scss}` is intentionally left untouched — the two systems
 * co-exist until later stages migrate every caller (spec §11).
 *
 * Scope model (spec §3):
 * - `window` — Portal into the originating window's `document.body`.
 *   Inert = the body's element children; backdrop covers the full
 *   window. Identical to modal-v2 today.
 * - `tab`   — mounts into the active tab's content root, supplied by a
 *   `TabModalScope` context. Inert = that tab's content; the tab bar +
 *   other tabs stay live. Falls back to `window` (with a console.warn)
 *   when no provider is present.
 * - `pane`  — mounts into a pane root supplied by a `PaneModalScope`
 *   context. Inert = that pane only. Built per spec §12.2 even though no
 *   caller uses it yet.
 *
 * Ported verbatim from modal-v2 (spec §8): focus trap via sentinel
 * spans, focus save/restore, ARIA labelling + the title-id context,
 * the pane-overlay clip, `prefers-reduced-motion` handling.
 *
 * Redesigned around `scope`: mount resolution (§7), scope-relative
 * inert + scroll lock (§5), the scope-aware stack (§6).
 *
 * New in §9: `closeOnBackdropClick={false}` no longer silently swallows
 * a backdrop click — it nudges the panel's `[data-modal-dismiss]`
 * control instead.
 *
 * Consumes design tokens from `theme.scss`:
 *   --z-modal, --shadow-modal, --shadow-focus-ring, --radius-lg,
 *   --motion-fast, --motion-base, --space-*
 */

import {
    createContext,
    createEffect,
    createSignal,
    createUniqueId,
    JSX,
    onCleanup,
    Show,
    useContext,
    type Accessor,
    type Component,
} from "solid-js";
import { Portal } from "solid-js/web";

import { usePaneOverlay } from "@/app/platform/pane-overlay";

import "./modal.scss";

// ── Scope ────────────────────────────────────────────────────────────────────

/** What region a modal locks. See spec §3. */
export type ModalScope = "window" | "tab" | "pane";

// ── Contexts ─────────────────────────────────────────────────────────────────
// `ModalTitleIdContext` shares the Modal's auto-generated title id with a
// nested `ModalHeader` so `aria-labelledby` on the dialog root resolves to
// the heading `ModalHeader` actually renders. Without it the two sides
// generate independent ids via `createUniqueId()` and never match,
// breaking the labelling contract. (Ported verbatim from modal-v2.)

const ModalTitleIdContext = createContext<string | undefined>(undefined);

/**
 * `TabModalScope` — a provider rendered inside a tab's content root
 * supplies the element a `scope="tab"` modal should mount into. The
 * slimmed-down successor to `TabModalLayer` (spec §7): instead of the
 * layer owning a request signal + render dispatch, it just exposes its
 * mount node and the unified `<Modal>` portals into it.
 *
 * Value is an accessor so the mount node can resolve lazily — the
 * provider may not have a ref on first render.
 */
export type ModalScopeMount = Accessor<HTMLElement | null | undefined>;

export const TabModalScope = createContext<ModalScopeMount | undefined>(undefined);

/**
 * `PaneModalScope` — same pattern as `TabModalScope`, for `scope="pane"`.
 * A pane/block root renders this provider; an in-pane `<Modal>` resolves
 * its mount node + inert region from it. No caller today (spec §3) — the
 * capability is built so the inert/stack design accounts for it.
 */
export const PaneModalScope = createContext<ModalScopeMount | undefined>(undefined);

// ── Pane airspace clip ───────────────────────────────────────────────────────
// Native browser-pane HWNDs composite above the HTML renderer, so CSS
// z-index can't stack a modal over a visible pane. `usePaneOverlay`
// registers the modal-root rect with the backend, which subtracts it
// from every pane's Win32 region so the pane's HWND paints transparent
// where the modal is. Rendered inside <Show> so registration is bound
// to the modal's open/close lifecycle, not its component instance.
// Full rationale: docs/specs/SPEC_MODAL_PANE_CLIP_2026_04_24.md.
// Ported verbatim from modal-v2's ModalPaneOverlayClip (spec §8).

const ModalPaneOverlayClip: Component<{ getEl: Accessor<HTMLElement | null | undefined> }> = (p) => {
    usePaneOverlay(p.getEl);
    return null;
};

// ── Modal stack (spec §6) ────────────────────────────────────────────────────
// Module-level so every Modal instance shares it. Unlike modal-v2's flat
// z-order stack, each entry records the modal's `scope` and `lockEl` so
// ESC / backdrop can reason about scope containment.
//
// The "reachable topmost" is the highest-stacked modal NOT contained
// within a higher modal's lock region. Modals whose lock regions don't
// overlap (two pane modals in different panes; a pane modal + a tab
// modal in another tab) coexist — each stays independently reachable.

interface StackEntry {
    id: string;
    scope: ModalScope;
    /** The element this modal locks (its backdrop / inert region). */
    lockEl: HTMLElement;
    close: () => void;
}

const stack: StackEntry[] = [];

const push = (entry: StackEntry): void => {
    stack.push(entry);
};

const remove = (id: string): void => {
    const idx = stack.findIndex((e) => e.id === id);
    if (idx >= 0) stack.splice(idx, 1);
};

/**
 * True when `inner`'s lock region is covered by `outer`'s — i.e. `outer`
 * is a higher modal whose backdrop blocks interaction with `inner`. A
 * region is covered if `outer.lockEl` contains `inner.lockEl`, or the two
 * resolve to the same node (two window modals share `document.body`).
 */
function covers(outer: StackEntry, inner: StackEntry): boolean {
    if (outer.lockEl === inner.lockEl) return true;
    return outer.lockEl.contains(inner.lockEl);
}

/**
 * The modal a global ESC / backdrop interaction should act on: the
 * highest-stacked modal not contained within any *higher* modal's lock
 * region. Modals shadowed by a higher overlapping modal are skipped.
 */
function reachableTopmost(): StackEntry | undefined {
    // Walk from the top down; the first entry not covered by anything
    // strictly above it is the reachable topmost.
    for (let i = stack.length - 1; i >= 0; i--) {
        const candidate = stack[i];
        let shadowed = false;
        for (let j = i + 1; j < stack.length; j++) {
            if (covers(stack[j], candidate)) {
                shadowed = true;
                break;
            }
        }
        if (!shadowed) return candidate;
    }
    return undefined;
}

/** True when `self` is the modal a global ESC / backdrop should target. */
function isReachableTopmost(self: StackEntry): boolean {
    return reachableTopmost()?.id === self.id;
}

// ── Per-region scroll + inert lock (spec §5) ────────────────────────────────
// modal-v2 inerts the whole document body unconditionally. The unified
// system inerts only the *lock region's* element children that aren't a
// modal root, and scroll-locks only that region.
//
// Reference-counted, keyed per lock-region element (a WeakMap keyed by
// the region node) — so stacked modals sharing a region release the lock
// only when the last one closes, while modals in disjoint regions never
// interfere. The window-scope region is `document.body`, so window-modal
// stacking behaves exactly as modal-v2's per-document lock did.

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
 * document body — identical to modal-v2. For tab/pane scope only that
 * region's children are inerted, leaving the rest of the page live.
 */
function acquireRegionLock(region: HTMLElement): void {
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
function releaseRegionLock(region: HTMLElement): void {
    const state = regionLocks.get(region);
    if (!state) return;
    state.openCount--;
    if (state.openCount > 0) return;
    for (const el of state.inertSiblings) el.removeAttribute("inert");
    region.style.overflow = state.previousOverflow;
    regionLocks.delete(region);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const FOCUSABLE_SELECTOR = [
    "input:not([disabled])",
    "textarea:not([disabled])",
    "select:not([disabled])",
    "button:not([disabled])",
    "a[href]",
    "[tabindex]:not([tabindex='-1'])",
].join(",");

function firstFocusable(root: HTMLElement): HTMLElement | null {
    return root.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
}

function lastFocusable(root: HTMLElement): HTMLElement | null {
    const nodes = root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
    return nodes.length ? nodes[nodes.length - 1] : null;
}

/**
 * Resolve the document the modal should mount into. Uses the currently
 * focused element's `ownerDocument` so a modal opened from a click in
 * the N-th CEF window mounts into that window's DOM, not the main
 * window's. (Ported verbatim from modal-v2 — used for `window` scope.)
 */
function resolveMountDocument(): Document {
    const active = typeof document !== "undefined" ? document.activeElement : null;
    return active?.ownerDocument ?? document;
}

// ── Cancel nudge (spec §9) ──────────────────────────────────────────────────
// When `closeOnBackdropClick` is false, a backdrop click must NOT close
// the modal — instead it nudges the panel's primary dismiss affordance.
// We find `[data-modal-dismiss]` inside the panel, add the
// `modal-dismiss--nudge` class, and remove it on `animationend` so a
// later click re-triggers the keyframe. No-op when the panel has no
// dismiss control. The reduced-motion CSS variant is still a (brief,
// non-moving) animation so `animationend` reliably fires.

const NUDGE_CLASS = "modal-dismiss--nudge";

function nudgeDismissControl(panel: HTMLElement | undefined): void {
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

// ── Modal ────────────────────────────────────────────────────────────────────

export interface ModalProps {
    open: boolean;
    onClose: () => void;
    /**
     * What region the modal locks. See spec §3. Default `"window"`.
     * - `window` — portals to the window body; locks the whole window.
     * - `tab`    — mounts into the active tab's content (TabModalScope).
     * - `pane`   — mounts into a pane root (PaneModalScope).
     * `tab`/`pane` fall back to `window` with a console.warn when no
     * matching scope provider is present.
     */
    scope?: ModalScope;
    /** Backdrop click closes. Default `true`. When `false`, a backdrop
     *  click nudges the panel's `[data-modal-dismiss]` control instead
     *  of dismissing (spec §9). */
    closeOnBackdropClick?: boolean;
    /** ESC closes. Default `true`. ESC always targets the reachable
     *  topmost modal regardless of `closeOnBackdropClick`. */
    closeOnEscape?: boolean;
    /** Width preset. `fit` = auto. Default `md`. */
    size?: "sm" | "md" | "lg" | "xl" | "fit";
    /** Vertical placement of the panel. `center` (default) centers
     *  with the grid; `top` anchors near the top of the region —
     *  matches command-palette-style surfaces that drop down from
     *  the top of the screen. */
    placement?: "center" | "top";
    /** Optional extra class on the panel — lets a caller apply
     *  component-specific layout without sidestepping the primitive. */
    panelClass?: string;
    /** Renders an X close button in the top-right corner of the panel.
     *  Clicking it invokes `onClose`. The X carries `data-modal-dismiss`
     *  so a rejected backdrop click nudges it (spec §9). */
    showCloseButton?: boolean;
    /** Override aria-labelledby. By default resolves from a nested ModalHeader. */
    ariaLabel?: string;
    ariaLabelledBy?: string;
    ariaDescribedBy?: string;
    /** Element (or accessor) to focus on open. Defaults to the first focusable. */
    initialFocus?: HTMLElement | (() => HTMLElement | null);
    children: JSX.Element;
}

export const Modal: Component<ModalProps> = (props) => {
    const id = createUniqueId();
    const defaultTitleId = `modal-title-${id}`;

    // Resolve the scope contexts at component-creation time (must be
    // called during the synchronous render of a tracking scope). The
    // accessors themselves are read lazily inside `openModal`.
    const tabMount = useContext(TabModalScope);
    const paneMount = useContext(PaneModalScope);

    let panelRef: HTMLDivElement | undefined;
    let rootRef: HTMLDivElement | undefined;
    let previousFocus: HTMLElement | null = null;

    // Cached on open so close targets exactly what open acquired, even
    // if focus / context moved meanwhile.
    let lockRegion: HTMLElement | null = null;
    let mountNode: HTMLElement | null = null;
    let stackEntry: StackEntry | null = null;

    const [mounted, setMounted] = createSignal(false);
    // The node the Portal mounts into. Set by `openModal` once the scope
    // is resolved; `<Portal mount>` is read after `mounted()` flips true.
    const [portalMount, setPortalMount] = createSignal<HTMLElement | null>(null);
    // Resolved scope — may differ from `props.scope` when a tab/pane modal
    // falls back to window (no provider). Drives `data-scope` so the CSS
    // positioning matches where the modal actually mounted.
    const [resolvedScope, setResolvedScope] = createSignal<ModalScope>("window");

    /**
     * Resolve the scope this modal renders into. Returns the Portal
     * mount node + the lock region (the element whose children get
     * inerted and which the backdrop is sized to).
     *
     * - `window`: mount = window body; lock region = window body.
     * - `tab`/`pane`: mount = lock region = the context-supplied node.
     *   When no provider is present (or it has no node yet) we fall
     *   back to window scope with a console.warn — an un-hosted scoped
     *   modal is a wiring bug, not a crash.
     */
    const resolveScope = (): { scope: ModalScope; mount: HTMLElement; region: HTMLElement } => {
        const requested = props.scope ?? "window";

        if (requested === "tab") {
            const node = tabMount?.();
            if (node) return { scope: "tab", mount: node, region: node };
            console.warn(
                "[modal] scope=\"tab\" used with no <TabModalScope> provider (or it has no " +
                    "mount node yet) — falling back to scope=\"window\".",
            );
        } else if (requested === "pane") {
            const node = paneMount?.();
            if (node) return { scope: "pane", mount: node, region: node };
            console.warn(
                "[modal] scope=\"pane\" used with no <PaneModalScope> provider (or it has no " +
                    "mount node yet) — falling back to scope=\"window\".",
            );
        }

        const body = resolveMountDocument().body;
        return { scope: "window", mount: body, region: body };
    };

    // Track `open` changes to run the open/close lifecycle.
    createEffect(() => {
        if (props.open && !mounted()) {
            openModal();
        } else if (!props.open && mounted()) {
            closeModal();
        }
    });

    onCleanup(() => {
        if (mounted()) closeModal();
    });

    const openModal = (): void => {
        previousFocus = (document.activeElement as HTMLElement) ?? null;

        const resolved = resolveScope();
        mountNode = resolved.mount;
        lockRegion = resolved.region;
        setPortalMount(mountNode);
        setResolvedScope(resolved.scope);

        // Acquire the per-region scroll + inert lock. Reference-counted
        // per lock-region element: the first modal in a region performs
        // the real lock; later modals just bump the count so the lock
        // stays active until the *last* modal in that region closes.
        acquireRegionLock(lockRegion);

        // Register in the scope-aware stack so ESC / backdrop dispatch
        // to the reachable topmost.
        stackEntry = { id, scope: resolved.scope, lockEl: lockRegion, close: props.onClose };
        push(stackEntry);

        setMounted(true);

        // Focus resolution — next frame so the Portal has rendered.
        requestAnimationFrame(() => {
            if (!panelRef) return;
            let target: HTMLElement | null = null;
            if (typeof props.initialFocus === "function") {
                target = props.initialFocus();
            } else if (props.initialFocus) {
                target = props.initialFocus;
            }
            if (!target) target = firstFocusable(panelRef);
            (target ?? panelRef).focus();
        });
    };

    const closeModal = (): void => {
        remove(id);
        stackEntry = null;

        // Release the per-region lock. If this was the last modal in
        // its region, the cleanup also restores scroll and clears inert.
        if (lockRegion) releaseRegionLock(lockRegion);
        lockRegion = null;
        mountNode = null;

        setMounted(false);
        setPortalMount(null);

        // Restore focus on the next tick so Solid's cleanup has finished
        // and the previously-focused element isn't immediately moved by
        // an unrelated reactive update.
        queueMicrotask(() => {
            if (previousFocus && previousFocus.isConnected && typeof previousFocus.focus === "function") {
                previousFocus.focus();
            }
            previousFocus = null;
        });
    };

    // Key handling. ESC fires on the *reachable topmost* — modals lower
    // in the stack, or shadowed by a higher overlapping modal, stay open.
    const handleKeyDown = (ev: KeyboardEvent): void => {
        if (ev.key !== "Escape") return;
        if (props.closeOnEscape === false) return;
        if (!stackEntry || !isReachableTopmost(stackEntry)) return;
        ev.preventDefault();
        ev.stopPropagation();
        props.onClose();
    };

    // Backdrop handling. Only acts when this modal is the reachable
    // topmost. When `closeOnBackdropClick` is false the click does not
    // dismiss — it nudges the panel's `[data-modal-dismiss]` control.
    const handleBackdropClick = (ev: MouseEvent): void => {
        if (!stackEntry || !isReachableTopmost(stackEntry)) return;
        // Only the backdrop itself, not a click bubbling up from the
        // panel. The panel is a sibling of the backdrop, so the simplest
        // correct test is `target === currentTarget`.
        if (ev.target !== ev.currentTarget) return;
        if (props.closeOnBackdropClick === false) {
            nudgeDismissControl(panelRef);
            return;
        }
        props.onClose();
    };

    // Sentinel focus trap. Focusing a sentinel bounces focus to the
    // opposite end of the panel so Tab and Shift+Tab both wrap inside
    // the dialog without escaping into the region behind. (Ported.)
    const onSentinelStartFocus = (): void => {
        if (!panelRef) return;
        (lastFocusable(panelRef) ?? panelRef).focus();
    };

    const onSentinelEndFocus = (): void => {
        if (!panelRef) return;
        (firstFocusable(panelRef) ?? panelRef).focus();
    };

    // ARIA labelling precedence: only one of aria-label / aria-labelledby
    // should be set. `aria-labelledby` wins over `aria-label` per the
    // ARIA spec, so sending both would ignore the caller's explicit
    // `ariaLabel`. Fall through: explicit labelledby → explicit label →
    // auto-wired via the ModalHeader's context-shared id. (Ported.)
    const labelledById = (): string | undefined => {
        if (props.ariaLabelledBy) return props.ariaLabelledBy;
        if (props.ariaLabel) return undefined; // label wins when no labelledby
        return defaultTitleId;
    };

    return (
        <Show when={mounted() && portalMount()}>
            {(mount) => (
                <Portal mount={mount()}>
                    <ModalTitleIdContext.Provider value={defaultTitleId}>
                        <div
                            ref={rootRef}
                            class="modal-root"
                            data-scope={resolvedScope()}
                            data-placement={props.placement ?? "center"}
                            role="dialog"
                            aria-modal="true"
                            aria-label={props.ariaLabelledBy ? undefined : props.ariaLabel}
                            aria-labelledby={labelledById()}
                            aria-describedby={props.ariaDescribedBy}
                            tabIndex={-1}
                            onKeyDown={handleKeyDown}
                        >
                            <ModalPaneOverlayClip getEl={() => rootRef} />
                            <div class="modal-backdrop" onClick={handleBackdropClick} />
                            <span
                                class="modal-focus-sentinel"
                                tabindex="0"
                                aria-hidden="true"
                                onFocus={onSentinelStartFocus}
                            />
                            <div
                                ref={panelRef}
                                class={`modal-panel ${props.panelClass ?? ""}`}
                                data-size={props.size ?? "md"}
                                tabIndex={-1}
                            >
                                <Show when={props.showCloseButton}>
                                    <button
                                        type="button"
                                        class="modal-panel-close-btn"
                                        aria-label="Close"
                                        data-modal-dismiss
                                        onClick={() => props.onClose()}
                                    >
                                        {"✕"}
                                    </button>
                                </Show>
                                {props.children}
                            </div>
                            <span
                                class="modal-focus-sentinel"
                                tabindex="0"
                                aria-hidden="true"
                                onFocus={onSentinelEndFocus}
                            />
                        </div>
                    </ModalTitleIdContext.Provider>
                </Portal>
            )}
        </Show>
    );
};

// ── Subcomponents ────────────────────────────────────────────────────────────

export interface ModalHeaderProps {
    title: string;
    description?: string;
    /** Override id (rarely needed — Modal auto-wires `aria-labelledby`). */
    id?: string;
}

export const ModalHeader: Component<ModalHeaderProps> = (props) => {
    // When rendered inside a <Modal>, inherit the title id the Modal
    // wired into `aria-labelledby`. When used standalone, fall back
    // to a freshly-generated id so we still get a valid element id.
    const contextTitleId = useContext(ModalTitleIdContext);
    const fallbackId = createUniqueId();
    const resolvedId = () => props.id ?? contextTitleId ?? fallbackId;
    return (
        <header class="modal-panel-header">
            <h2 class="modal-panel-title" id={resolvedId()}>
                {props.title}
            </h2>
            <Show when={props.description}>
                <p class="modal-panel-description">{props.description}</p>
            </Show>
        </header>
    );
};

export const ModalBody: Component<{ children: JSX.Element; class?: string }> = (props) => (
    <div class={`modal-panel-body ${props.class ?? ""}`}>{props.children}</div>
);

export const ModalFooter: Component<{ children: JSX.Element; class?: string }> = (props) => (
    <footer class={`modal-panel-footer ${props.class ?? ""}`}>{props.children}</footer>
);

// ── ConfirmModal preset ──────────────────────────────────────────────────────
// Common "title + body + Cancel / Confirm" pattern. `destructive` flips
// the confirm button colour to red and routes initial focus to Cancel
// so a stray Enter doesn't delete the thing the user was about to
// double-check. Composes around `Modal` — no new primitive concepts.
//
// The Cancel button carries `data-modal-dismiss` so a rejected backdrop
// click (`closeOnBackdropClick={false}`) nudges it (spec §9).

export interface ConfirmModalProps {
    open: boolean;
    title: string;
    description?: string;
    /** Locks the window by default; pass `tab`/`pane` to scope it. */
    scope?: ModalScope;
    /** Rendered inside the body above the footer buttons. */
    children?: JSX.Element;
    confirmLabel?: string;             // default "OK"
    cancelLabel?: string;              // default "Cancel"
    /** Destructive confirmation — red button + initial focus on Cancel. */
    destructive?: boolean;
    onConfirm: () => void | Promise<void>;
    onCancel: () => void;
}

export const ConfirmModal: Component<ConfirmModalProps> = (props) => {
    const [pending, setPending] = createSignal(false);
    let cancelBtnRef: HTMLButtonElement | undefined;

    const handleConfirm = async () => {
        if (pending()) return;
        try {
            setPending(true);
            await props.onConfirm();
        } finally {
            setPending(false);
        }
    };

    return (
        <Modal
            open={props.open}
            scope={props.scope}
            onClose={() => { if (!pending()) props.onCancel(); }}
            closeOnBackdropClick={!pending()}
            closeOnEscape={!pending()}
            size="sm"
            initialFocus={() => (props.destructive ? (cancelBtnRef ?? null) : null)}
        >
            <ModalHeader title={props.title} description={props.description} />
            <Show when={props.children}>
                <ModalBody>{props.children}</ModalBody>
            </Show>
            <ModalFooter>
                <button
                    ref={cancelBtnRef}
                    type="button"
                    class="modal-btn modal-btn--cancel"
                    data-modal-dismiss
                    onClick={() => { if (!pending()) props.onCancel(); }}
                    disabled={pending()}
                >
                    {props.cancelLabel ?? "Cancel"}
                </button>
                <button
                    type="button"
                    class={`modal-btn modal-btn--confirm${props.destructive ? " modal-btn--destructive" : ""}`}
                    onClick={() => void handleConfirm()}
                    disabled={pending()}
                >
                    {pending() ? "…" : (props.confirmLabel ?? "OK")}
                </button>
            </ModalFooter>
        </Modal>
    );
};
