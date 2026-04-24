// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Modal v2 — accessible, multi-window-aware, stackable.
 *
 * Implements PR 1 of SPEC_ROBUST_MODAL_SYSTEM_2026_04_23. Retires
 * `element/modal.tsx` and the `modals/` wrapper once callers migrate
 * (see spec §6 PRs 3–5).
 *
 * Behaviour:
 * - `role="dialog"` + `aria-modal="true"` + generated `aria-labelledby`.
 * - Portal mounts into the originating window's document (multi-window
 *   aware via `document.activeElement?.ownerDocument`).
 * - Body scroll locked while any modal is open.
 * - Background siblings of the modal root receive `inert` so screen
 *   readers and keyboard focus don't escape.
 * - Focus saved on open, restored on close. Trap via sentinel spans.
 * - ESC closes topmost; modal stack coordinates stacking.
 * - Backdrop click closes (opt-out via `closeOnBackdropClick={false}`).
 * - Animations honour `prefers-reduced-motion`.
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

import "./modal-v2.scss";

// ── Context ──────────────────────────────────────────────────────────────────
// Shares the Modal's auto-generated title id with a nested ModalHeader
// so `aria-labelledby` on the dialog root resolves to the heading that
// ModalHeader actually renders. Without this context the two sides
// generate independent ids via `createUniqueId()` and never match,
// breaking the labelling contract.

const ModalTitleIdContext = createContext<string | undefined>(undefined);

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

// ── Modal stack ──────────────────────────────────────────────────────────────
// Module-level so multiple Modal instances share it. ESC and backdrop
// dispatch only to the topmost entry.

interface StackEntry {
    id: string;
    close: () => void;
}

const stack: StackEntry[] = [];

const topmost = (): StackEntry | undefined => stack[stack.length - 1];

const push = (entry: StackEntry): void => {
    stack.push(entry);
};

const remove = (id: string): void => {
    const idx = stack.findIndex((e) => e.id === id);
    if (idx >= 0) stack.splice(idx, 1);
};

// ── Per-document scroll + inert lock ────────────────────────────────────────
// Reference-counted so stacked modals (or modals closing out of order)
// don't release the lock prematurely. Codex flagged the unconditional
// release on PR #511; the shared state below handles stacking cleanly.

interface DocumentLockState {
    openCount: number;
    previousOverflow: string;
    inertSiblings: HTMLElement[];
}

const docLocks = new WeakMap<Document, DocumentLockState>();

function acquireDocumentLock(doc: Document): void {
    const existing = docLocks.get(doc);
    if (existing) {
        existing.openCount++;
        return;
    }
    const state: DocumentLockState = {
        openCount: 1,
        previousOverflow: doc.body.style.overflow,
        inertSiblings: [],
    };
    doc.body.style.overflow = "hidden";
    if ("inert" in HTMLElement.prototype) {
        for (const child of Array.from(doc.body.children) as HTMLElement[]) {
            if (!child.classList.contains("modal-root") && !child.hasAttribute("inert")) {
                child.setAttribute("inert", "");
                state.inertSiblings.push(child);
            }
        }
    }
    docLocks.set(doc, state);
}

function releaseDocumentLock(doc: Document): void {
    const state = docLocks.get(doc);
    if (!state) return;
    state.openCount--;
    if (state.openCount > 0) return;
    for (const el of state.inertSiblings) el.removeAttribute("inert");
    doc.body.style.overflow = state.previousOverflow;
    docLocks.delete(doc);
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
 * Resolve the document the modal should mount into. Uses the
 * currently focused element's `ownerDocument` so a modal opened
 * from a click in the N-th CEF window mounts into that window's
 * DOM, not the main window's.
 */
function resolveMountDocument(): Document {
    const active = typeof document !== "undefined" ? document.activeElement : null;
    return active?.ownerDocument ?? document;
}

// ── Modal ────────────────────────────────────────────────────────────────────

export interface ModalProps {
    open: boolean;
    onClose: () => void;
    /** Backdrop click closes. Default `true`. */
    closeOnBackdropClick?: boolean;
    /** ESC closes. Default `true`. */
    closeOnEscape?: boolean;
    /** Width preset. `fit` = auto. Default `md`. */
    size?: "sm" | "md" | "lg" | "xl" | "fit";
    /** Vertical placement of the panel. `center` (default) centers
     *  with the grid; `top` anchors near the top of the viewport —
     *  matches command-palette-style surfaces that drop down from
     *  the top of the screen. */
    placement?: "center" | "top";
    /** Optional extra class on the panel — lets a caller apply
     *  component-specific layout without sidestepping the primitive. */
    panelClass?: string;
    /** Renders an X close button in the top-right corner of the panel.
     *  Clicking it invokes `onClose`. Useful for informational modals
     *  where the only disposition is "dismiss" — AboutModal, etc. */
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

    let panelRef: HTMLDivElement | undefined;
    let rootRef: HTMLDivElement | undefined;
    let previousFocus: HTMLElement | null = null;
    // Cache the document we acquired the lock on so release targets the
    // same body, even if focus moved across CEF windows meanwhile.
    // Per-doc scroll + inert state lives in the shared `docLocks` map
    // (reference-counted — handles stacked / out-of-order closes
    // correctly; see Codex P1 on PR #511).
    let mountDoc: Document | null = null;

    const [mounted, setMounted] = createSignal(false);

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

        // Acquire the per-document scroll + inert lock. Reference-
        // counted: the first modal in a document performs the actual
        // lock; later modals just bump the count so the lock stays
        // active until the *last* modal closes.
        mountDoc = resolveMountDocument();
        acquireDocumentLock(mountDoc);

        // Register in the stack so ESC / backdrop dispatch to topmost.
        push({ id, close: props.onClose });

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

        // Release the per-document lock. If this was the last modal
        // in its document, the lock's cleanup also restores scroll
        // and clears inert. Lower modals in a stack don't release
        // the lock until they're gone too.
        if (mountDoc) releaseDocumentLock(mountDoc);
        mountDoc = null;

        setMounted(false);

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

    // Key + backdrop handling. ESC only fires on the topmost — modals
    // lower in the stack stay open.
    const handleKeyDown = (ev: KeyboardEvent): void => {
        if (ev.key !== "Escape") return;
        if (props.closeOnEscape === false) return;
        if (topmost()?.id !== id) return;
        ev.preventDefault();
        ev.stopPropagation();
        props.onClose();
    };

    const handleBackdropClick = (ev: MouseEvent): void => {
        if (props.closeOnBackdropClick === false) return;
        if (topmost()?.id !== id) return;
        // Only the backdrop itself should close — clicks inside the
        // panel bubble through but the panel is a sibling, not a child
        // of the backdrop, so `target === backdrop` is the simplest test.
        if (ev.target === ev.currentTarget) {
            props.onClose();
        }
    };

    // Sentinel focus trap. Focusing a sentinel bounces focus to the
    // opposite end of the panel so Tab and Shift+Tab both wrap inside
    // the dialog without escaping into the page behind.
    const onSentinelStartFocus = (): void => {
        if (!panelRef) return;
        (lastFocusable(panelRef) ?? panelRef).focus();
    };

    const onSentinelEndFocus = (): void => {
        if (!panelRef) return;
        (firstFocusable(panelRef) ?? panelRef).focus();
    };

    // ARIA labelling precedence: only one of aria-label / aria-labelledby
    // should be set. `aria-labelledby` wins over `aria-label` per the ARIA
    // spec, so sending both would ignore the caller's explicit `ariaLabel`
    // prop. Fall through in order: explicit labelledby → explicit label →
    // auto-wired via the ModalHeader's context-shared id.
    const labelledById = (): string | undefined => {
        if (props.ariaLabelledBy) return props.ariaLabelledBy;
        if (props.ariaLabel) return undefined; // label wins when no labelledby
        return defaultTitleId;
    };

    return (
        <Show when={mounted()}>
            <Portal mount={resolveMountDocument().body}>
                <ModalTitleIdContext.Provider value={defaultTitleId}>
                    <div
                        ref={rootRef}
                        class="modal-root"
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
                                    onClick={() => props.onClose()}
                                >
                                    {"\u2715"}
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

export interface ConfirmModalProps {
    open: boolean;
    title: string;
    description?: string;
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
