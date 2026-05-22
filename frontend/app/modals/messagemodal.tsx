// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { Modal, ModalBody, ModalFooter } from "@/element/modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";

import type { JSX } from "solid-js";

const MessageModal = ({ children, close }: { children: JSX.Element } & ModalCloseProps) => {
    return (
        <Modal open={true} onClose={close} size="md" ariaLabel="Message">
            <ModalBody>{children}</ModalBody>
            <ModalFooter>
                <Button onClick={close}>Ok</Button>
            </ModalFooter>
        </Modal>
    );
};

MessageModal.displayName = "MessageModal";

export { MessageModal };
