// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Modal } from "@/app/modals/modal";
import { modalsModel } from "@/app/store/modalmodel";

import type { JSX } from "solid-js";
import "./messagemodal.scss";

const MessageModal = ({ children }: { children: JSX.Element }) => {
    function closeModal() {
        modalsModel.popModal();
    }

    return (
        <Modal class="message-modal" onOk={() => closeModal()} onClose={() => closeModal()}>
            {children}
        </Modal>
    );
};

MessageModal.displayName = "MessageModal";

export { MessageModal };
