// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Slim modal renderer (unified-modal-system stage 4).
//
// Iterates the flat signal-list of open modal entries (`modalsModel`) and
// renders each entry's component directly — no `displayName` string lookup,
// no keyed registry. Each component receives its own `close` handle so it can
// dismiss itself regardless of stack position.

import { setModalOpen } from "@/store/global";
import { modalsModel } from "@/store/modalmodel";
import { createEffect, For, type JSX } from "solid-js";

const ModalsRenderer = (): JSX.Element => {
    const modals = modalsModel.modalsAtom;

    createEffect(() => {
        setModalOpen(modals().length > 0);
    });

    return (
        <For each={modals()}>
            {(modal) => {
                const { Component } = modal;
                const close = () => modalsModel.closeModal(modal.id);
                return <Component {...modal.props} close={close} />;
            }}
        </For>
    );
};

export { ModalsRenderer };
