// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/button";
import { cn } from "@/util/util";
import clsx from "clsx";
import { type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import "./modal.scss";

interface ModalProps {
    children?: JSX.Element;
    okLabel?: string;
    cancelLabel?: string;
    class?: string;
    onClickBackdrop?: () => void;
    onOk?: () => void;
    onCancel?: () => void;
    onClose?: () => void;
}

const Modal = ({ children, class: className, cancelLabel, okLabel, onCancel, onOk, onClose, onClickBackdrop }: ModalProps) => {
    let divRef!: HTMLDivElement;
    const renderBackdrop = (onClick: () => void) => <div class="modal-backdrop" onClick={onClick}></div>;

    const renderFooter = () => {
        return onOk || onCancel;
    };

    return (
        <Portal mount={document.getElementById("main")}>
            <div class="modal-wrapper">
                {renderBackdrop(onClickBackdrop)}
                <div ref={divRef} class={clsx(`modal`, className)}>
                    <Button className="grey ghost modal-close-btn" onClick={onClose} title="Close (ESC)">
                        <i class="fa-sharp fa-solid fa-xmark"></i>
                    </Button>
                    <div class="content-wrapper">
                        <ModalContent>{children}</ModalContent>
                    </div>
                    {renderFooter() && (
                        <ModalFooter onCancel={onCancel} onOk={onOk} cancelLabel={cancelLabel} okLabel={okLabel} />
                    )}
                </div>
            </div>
        </Portal>
    );
};

interface ModalContentProps {
    children: JSX.Element;
}

function ModalContent({ children }: ModalContentProps) {
    return <div class="modal-content">{children}</div>;
}

interface ModalFooterProps {
    okLabel?: string;
    cancelLabel?: string;
    onOk?: () => void;
    onCancel?: () => void;
}

const ModalFooter = ({ onCancel, onOk, cancelLabel = "Cancel", okLabel = "Ok" }: ModalFooterProps) => {
    return (
        <footer class="modal-footer">
            {onCancel && (
                <Button className="grey ghost" onClick={onCancel}>
                    {cancelLabel}
                </Button>
            )}
            {onOk && <Button onClick={onOk}>{okLabel}</Button>}
        </footer>
    );
};

interface FlexiModalProps {
    children?: JSX.Element;
    class?: string;
    onClickBackdrop?: () => void;
}

const FlexiModal = ({ children, class: className, onClickBackdrop }: FlexiModalProps) => {
    let divRef!: HTMLDivElement;
    const renderBackdrop = (onClick: () => void) => <div class="modal-backdrop" onClick={onClick}></div>;

    return (
        <Portal mount={document.getElementById("main")}>
            <div class="modal-wrapper">
                {renderBackdrop(onClickBackdrop)}
                <div class={cn("modal pt-6 px-4 pb-4", className)} ref={divRef}>
                    {children}
                </div>
            </div>
        </Portal>
    );
};

(FlexiModal as any).Content = ModalContent;
(FlexiModal as any).Footer = ModalFooter;

export { FlexiModal, Modal };
