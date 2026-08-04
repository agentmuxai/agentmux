// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Modal — the canonical dialog primitive.
 *
 * One modal system for the whole app. The `scope` prop selects what
 * region the modal *locks* — the window, a tab, or a single pane.
 * Mount point, backdrop extent, `inert` boundary, scroll lock, and
 * the modal stack are all consequences of that scope.
 *
 * Scope model:
 * - `window` — Portal into the originating window's `document.body`.
 *   Inert = the body's element children; backdrop covers the full
 *   window.
 * - `tab`   — mounts into the active tab's content root, supplied by a
 *   `TabModalScope` context. Inert = that tab's content; the tab bar +
 *   other tabs stay live. Falls back to `window` (with a console.warn)
 *   when no provider is present.
 * - `pane`  — mounts into a pane root supplied by a `PaneModalScope`
 *   context. Inert = that pane only. The infrastructure is wired even
 *   though no in-tree caller uses it yet — pane-scoped modals (e.g. a
 *   launch flow that should only lock its own agent pane) plug in by
 *   adding a `<PaneModalScope.Provider>` at the pane root and passing
 *   `scope="pane"`.
 *
 * Features:
 * - Focus trap via sentinel spans, focus save/restore on close.
 * - ARIA labelling — auto-generated title id shared between the
 *   dialog root and a nested `ModalHeader` via `ModalTitleIdContext`.
 * - Pane-overlay clip — registers the modal's rect with the backend
 *   so native browser-pane HWNDs paint transparent under the modal
 *   (`SPEC_MODAL_PANE_CLIP_2026_04_24.md`).
 * - `prefers-reduced-motion` honored — entrance/exit animations
 *   suppressed when set.
 * - `closeOnBackdropClick={false}` nudges the panel's
 *   `[data-modal-dismiss]` control instead of silently swallowing
 *   the click.
 *
 * History (relevant when reading older PRs / specs): this file is the
 * descendant of two prior implementations — `modal.tsx` (v1, single
 * Portal into `document.body`) and `modal-v2.tsx` (added the chrome
 * slot system + paint gate). Both are gone; the surviving file is the
 * canonical one. Older specs reference "modal-v2"; treat that as a
 * pointer to this file. Design rationale for the scope axis lives in
 * `SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md`.
 *
 * Consumes design tokens from `theme.scss`:
 *   --z-modal, --shadow-modal, --shadow-focus-ring, --radius-lg,
 *   --motion-fast, --motion-base, --space-*
 *
 * This file used to also carry the modal stack, region-lock manager,
 * focus-trap DOM utilities, backdrop-dismiss nudge helper, the
 * ModalHeader/ModalBody/ModalFooter subcomponents, and ConfirmModal.
 * Those are now split out (./modal-stack, ./modal-region-lock,
 * ./modal-focus-trap, ./modal-dismiss-nudge, ./modal-parts,
 * ./confirm-modal) — re-exported below so every external import path
 * keeps resolving unchanged.
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

import { nudgeDismissControl } from "./modal-dismiss-nudge";
import { firstFocusable, lastFocusable } from "./modal-focus-trap";
import { ModalTitleIdContext } from "./modal-parts";
import { acquireRegionLock, releaseRegionLock } from "./modal-region-lock";
import { push, remove, isReachable, type StackEntry } from "./modal-stack";

import "./modal.scss";

export { ModalHeader, ModalBody, ModalFooter } from "./modal-parts";
export { ConfirmModal } from "./confirm-modal";

// ── Scope ────────────────────────────────────────────────────────────────────

/** What region a modal locks. See spec §3. */
export type ModalScope = "window" | "tab" | "pane";

// ── Contexts ─────────────────────────────────────────────────────────────────

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

const ModalPaneOverlayClip: Component<{ getEl: Accessor<HTMLElement | null | undefined> }> = (p) => {
    usePaneOverlay(p.getEl);
    return null;
};

/**
 * Resolve the document the modal should mount into. Uses the currently
 * focused element's `ownerDocument` so a modal opened from a click in
 * the N-th CEF window mounts into that window's DOM, not the main
 * window's. Used for `window` scope.
 */
function resolveMountDocument(): Document {
    const active = typeof document !== "undefined" ? document.activeElement : null;
    return active?.ownerDocument ?? document;
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
    /** Called with the mounted `.modal-root` element (and `null` on unmount).
     *  For a caller that needs to observe/query the modal's own rendered
     *  content directly — e.g. ModalLayer's backdrop-dismissibility check —
     *  without assuming exactly where in the DOM tree `<Portal>` places it
     *  relative to whatever ref the caller already holds (it's not always a
     *  direct child — Portal/Show wrapping can interpose nodes). */
    rootRef?: (el: HTMLDivElement | null) => void;
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
        if (!stackEntry || !isReachable(stackEntry)) return;
        ev.preventDefault();
        ev.stopPropagation();
        props.onClose();
    };

    // Backdrop handling. Only acts when this modal is the reachable
    // topmost. When `closeOnBackdropClick` is false the click does not
    // dismiss — it nudges the panel's `[data-modal-dismiss]` control.
    const handleBackdropClick = (ev: MouseEvent): void => {
        if (!stackEntry || !isReachable(stackEntry)) return;
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
                            ref={(el) => {
                                rootRef = el;
                                props.rootRef?.(el);
                                onCleanup(() => props.rootRef?.(null));
                            }}
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
