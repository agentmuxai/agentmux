// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { setModalOpen } from "@/store/global";
import { modalsModel } from "@/store/modalmodel";
import { createEffect, For, type JSX } from "solid-js";
import { getModalComponent } from "./modalregistry";

const ModalsRenderer = (): JSX.Element => {
    const modals = modalsModel.modalsAtom;

    createEffect(() => {
        setModalOpen(modals().length > 0);
    });

    return (
        <For each={modals()}>
            {(modal) => {
                const ModalComponent = getModalComponent(modal.displayName);
                if (!ModalComponent) return null;
                return <ModalComponent {...modal.props} />;
            }}
        </For>
    );
};

export { ModalsRenderer };
