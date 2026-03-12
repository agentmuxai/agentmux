// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { JSX, Show } from "solid-js";

import "./modal.scss";

interface ModalProps {
    id?: string;
    children?: JSX.Element;
    onClickOut: () => void;
}

function Modal(props: ModalProps): JSX.Element {
    const id = props.id ?? "modal";

    const handleOutsideClick = (e: MouseEvent) => {
        if (typeof props.onClickOut === "function" && (e.target as Element).className === "modal-container") {
            props.onClickOut();
        }
    };

    return (
        <div class="modal-container" onClick={handleOutsideClick}>
            <dialog id={id} class="modal">
                {props.children}
            </dialog>
        </div>
    );
}

interface ModalContentProps {
    children?: JSX.Element;
}

function ModalContent(props: ModalContentProps): JSX.Element {
    return <div class="modal-content">{props.children}</div>;
}

interface ModalHeaderProps {
    title: JSX.Element | string;
    description?: string;
}

function ModalHeader(props: ModalHeaderProps): JSX.Element {
    return (
        <header class="modal-header">
            {typeof props.title === "string" ? <h3 class="modal-title">{props.title}</h3> : props.title}
            <Show when={props.description}>
                <p>{props.description}</p>
            </Show>
        </header>
    );
}

interface ModalFooterProps {
    children?: JSX.Element;
}

function ModalFooter(props: ModalFooterProps): JSX.Element {
    return <footer class="modal-footer">{props.children}</footer>;
}

interface WaveModalProps {
    title: string;
    description?: string;
    id?: string;
    onSubmit: () => void;
    onCancel: () => void;
    buttonLabel?: string;
    children?: JSX.Element;
}

function WaveModal(props: WaveModalProps): JSX.Element {
    const buttonLabel = props.buttonLabel ?? "Ok";
    return (
        <Modal onClickOut={props.onCancel}>
            <ModalHeader title={props.title} description={props.description} />
            <ModalContent>{props.children}</ModalContent>
            <ModalFooter>
                <Button onClick={props.onSubmit}>{buttonLabel}</Button>
            </ModalFooter>
        </Modal>
    );
}

export { WaveModal };
