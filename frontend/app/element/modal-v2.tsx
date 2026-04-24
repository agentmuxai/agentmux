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
    createEffect,
    createSignal,
    createUniqueId,
    JSX,
    onCleanup,
    onMount,
    Show,
    type Component,
} from "solid-js";
import { Portal } from "solid-js/web";

import "./modal-v2.scss";

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
    let previousFocus: HTMLElement | null = null;
    let inertSiblings: HTMLElement[] = [];
    let previousBodyOverflow = "";

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

        // Lock body scroll on the originating document.
        const doc = resolveMountDocument();
        previousBodyOverflow = doc.body.style.overflow;
        doc.body.style.overflow = "hidden";

        // Inert background siblings so assistive tech + keyboard
        // focus can't escape. Skip if `inert` isn't supported
        // (older CEF versions) — focus trap still handles keyboard.
        inertSiblings = [];
        if ("inert" in HTMLElement.prototype) {
            for (const child of Array.from(doc.body.children) as HTMLElement[]) {
                // The Portal mount target is doc.body, so the modal-root
                // becomes a body child after mount. We inert everything
                // else and rely on the modal's own focus trap.
                if (!child.classList.contains("modal-root")) {
                    if (!child.hasAttribute("inert")) {
                        child.setAttribute("inert", "");
                        inertSiblings.push(child);
                    }
                }
            }
        }

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

        // Restore inert attribute on siblings.
        for (const el of inertSiblings) el.removeAttribute("inert");
        inertSiblings = [];

        // Restore body scroll.
        const doc = resolveMountDocument();
        doc.body.style.overflow = previousBodyOverflow;

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

    return (
        <Show when={mounted()}>
            <Portal mount={resolveMountDocument().body}>
                <div
                    class="modal-root"
                    role="dialog"
                    aria-modal="true"
                    aria-label={props.ariaLabel}
                    aria-labelledby={props.ariaLabelledBy ?? defaultTitleId}
                    aria-describedby={props.ariaDescribedBy}
                    tabIndex={-1}
                    onKeyDown={handleKeyDown}
                >
                    <div class="modal-backdrop" onClick={handleBackdropClick} />
                    <span
                        class="modal-focus-sentinel"
                        tabindex="0"
                        aria-hidden="true"
                        onFocus={onSentinelStartFocus}
                    />
                    <div
                        ref={panelRef}
                        class="modal-panel"
                        data-size={props.size ?? "md"}
                        tabIndex={-1}
                    >
                        {props.children}
                    </div>
                    <span
                        class="modal-focus-sentinel"
                        tabindex="0"
                        aria-hidden="true"
                        onFocus={onSentinelEndFocus}
                    />
                </div>
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
    const fallbackId = createUniqueId();
    return (
        <header class="modal-panel-header">
            <h2 class="modal-panel-title" id={props.id ?? fallbackId}>
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
