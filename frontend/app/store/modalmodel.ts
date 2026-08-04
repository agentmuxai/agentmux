// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Thin imperative modal API (unified-modal-system stage 4).
//
// Replaces the legacy `displayName`-keyed registry indirection. Modals are
// opened with a *direct component reference* via `openModal(Component, props)`;
// the store keeps a flat signal-list of open entries and the slim renderer
// (`modalsrenderer.tsx`) renders each entry's component. Each component itself
// renders a unified `<Modal scope="window">`.
//
// Every opened component receives a `close` prop — a per-entry close handle —
// so a modal dismisses *itself* without needing to know its stack position
// (the old `popModal()` always popped the top, which is wrong once two modals
// stack). `openModal` also returns the same handle for fire-and-forget callers.

import { createSignal, type JSX } from "solid-js";

/** Props every modal component opened via `openModal` receives. */
export interface ModalCloseProps {
    /** Closes *this* modal entry. Idempotent. */
    close: () => void;
}

interface ModalEntry {
    id: number;
    /** The modal component. Receives its own props plus an injected `close`. */
    Component: (props: any) => JSX.Element;
    /** Props passed to the component (without `close` — that is injected). */
    props: Record<string, any>;
}

/** Handle returned by `openModal` — lets the caller close the modal it opened. */
interface ModalHandle {
    /** Closes the modal. Idempotent — a no-op if already closed. */
    close: () => void;
}

class ModalsModel {
    private _modals: () => ModalEntry[];
    private _setModals: (v: ModalEntry[]) => void;
    private _nextId = 1;

    constructor() {
        const [get, set] = createSignal<ModalEntry[]>([]);
        this._modals = get;
        this._setModals = set;
    }

    /** Reactive accessor — call in a SolidJS component to get live modal list. */
    get modalsAtom() {
        return this._modals;
    }

    /** Closes the modal entry with `id` (if still present). Idempotent. */
    closeModal = (id: number): void => {
        this._setModals(this._modals().filter((m) => m.id !== id));
    };

    /**
     * Opens `Component` as a window-scoped modal. `Component` is rendered with
     * `props` plus an injected `close` callback. Returns a handle whose
     * `close()` dismisses exactly this modal (idempotent).
     */
    openModal = <P extends object>(
        Component: (props: P & ModalCloseProps) => JSX.Element,
        props?: P
    ): ModalHandle => {
        const id = this._nextId++;
        this._setModals([
            ...this._modals(),
            { id, Component: Component as (p: any) => JSX.Element, props: { ...(props ?? {}) } },
        ]);
        return { close: () => this.closeModal(id) };
    };

    /** Closes the topmost open modal, if any. Used by the global Escape key. */
    closeTopModal = (): void => {
        const modals = this._modals();
        if (modals.length > 0) {
            this.closeModal(modals[modals.length - 1].id);
        }
    };

    hasOpenModals(): boolean {
        return this._modals().length > 0;
    }

    /** True if a modal rendered by `Component` is currently open. */
    isModalOpen(Component: (props: any) => JSX.Element): boolean {
        return this._modals().some((modal) => modal.Component === Component);
    }
}

const modalsModel = new ModalsModel();

/**
 * Imperative helper — opens a window-scoped modal and returns a close handle.
 * Thin sugar over `modalsModel.openModal`; the preferred call-site entrypoint.
 */
const openModal = modalsModel.openModal;

export { modalsModel, openModal };
