// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Component, JSX, Show, createContext, createUniqueId, useContext } from "solid-js";

// ── Contexts ─────────────────────────────────────────────────────────────────
// `ModalTitleIdContext` shares the Modal's auto-generated title id with a
// nested `ModalHeader` so `aria-labelledby` on the dialog root resolves to
// the heading `ModalHeader` actually renders. Without it the two sides
// generate independent ids via `createUniqueId()` and never match,
// breaking the labelling contract.

export const ModalTitleIdContext = createContext<string | undefined>(undefined);

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
