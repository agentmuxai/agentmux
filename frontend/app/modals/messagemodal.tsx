// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { Modal, ModalBody, ModalFooter } from "@/element/modal-v2";
import { modalsModel } from "@/app/store/modalmodel";

import type { JSX } from "solid-js";

const MessageModal = ({ children }: { children: JSX.Element }) => {
    function closeModal() {
        modalsModel.popModal();
    }

    return (
        <Modal open={true} onClose={closeModal} size="md" ariaLabel="Message">
            <ModalBody>{children}</ModalBody>
            <ModalFooter>
                <Button onClick={closeModal}>Ok</Button>
            </ModalFooter>
        </Modal>
    );
};

MessageModal.displayName = "MessageModal";

export { MessageModal };
