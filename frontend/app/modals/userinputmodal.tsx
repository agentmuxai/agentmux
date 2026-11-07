// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { TextInput } from "@/app/element/ui";
import { Markdown } from "@/element/markdown";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/element/modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import * as keyutil from "@/util/keyutil";
import { fireAndForget } from "@/util/util";
import { createSignal, onCleanup, type JSX } from "solid-js";
import { UserInputService } from "../store/services";
import "./userinputmodal.scss";

type UserInputProps = UserInputRequest & ModalCloseProps;

function createUserInputState(props: UserInputProps) {
    const [responseText, setResponseText] = createSignal("");
    const [countdown, setCountdown] = createSignal(Math.floor(props.timeoutms / 1000));
    const checkbox: { el?: HTMLInputElement } = {};
    return { props, responseText, setResponseText, countdown, setCountdown, checkbox };
}

type UserInputState = ReturnType<typeof createUserInputState>;

function sendErrResponse(s: UserInputState): void {
    fireAndForget(() =>
        UserInputService.SendUserInputResponse({
            type: "userinputresp",
            requestid: s.props.requestid,
            errormsg: "Canceled by the user",
        })
    );
    s.props.close();
}

function sendText(s: UserInputState): void {
    fireAndForget(() =>
        UserInputService.SendUserInputResponse({
            type: "userinputresp",
            requestid: s.props.requestid,
            text: s.responseText(),
            checkboxstat: s.checkbox.el?.checked ?? false,
        })
    );
    s.props.close();
}

function sendConfirm(s: UserInputState, response: boolean): void {
    fireAndForget(() =>
        UserInputService.SendUserInputResponse({
            type: "userinputresp",
            requestid: s.props.requestid,
            confirm: response,
            checkboxstat: s.checkbox.el?.checked ?? false,
        })
    );
    s.props.close();
}

function submit(s: UserInputState): void {
    switch (s.props.responsetype) {
        case "text":
            sendText(s);
            break;
        case "confirm":
            sendConfirm(s, true);
            break;
    }
}

function negativeResponse(s: UserInputState): void {
    switch (s.props.responsetype) {
        case "text":
            sendErrResponse(s);
            break;
        case "confirm":
            sendConfirm(s, false);
            break;
    }
}

function onInputKeyDown(s: UserInputState, muxEvent: MuxKeyboardEvent): boolean {
    // Enter submits; ESC is handled by `<Modal>`'s closeOnEscape
    // which calls our onClose → sendErrResponse.
    if (keyutil.checkKeyPressed(muxEvent, "Enter")) {
        submit(s);
        return true;
    }
}

// Countdown timer using setInterval — fires an err response when it
// hits zero. Cleanup on unmount so a cancel/submit stops the tick
// AND clears the pending 300ms-delayed err-response so dismissing
// within that window doesn't double-fire sendErrResponse
// against a stale request (would pop a sibling modal).
function startCountdown(s: UserInputState): void {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    const intervalId: ReturnType<typeof setInterval> = setInterval(() => {
        s.setCountdown((prev) => {
            if (prev <= 1) {
                clearInterval(intervalId);
                timeoutId = setTimeout(() => sendErrResponse(s), 300);
                return 0;
            }
            return prev - 1;
        });
    }, 1000);
    onCleanup(() => {
        clearInterval(intervalId);
        if (timeoutId !== null) clearTimeout(timeoutId);
    });
}

function QueryText({ s }: { s: UserInputState }): JSX.Element {
    if (s.props.markdown) {
        return <Markdown text={s.props.querytext} class="userinput-markdown" /> as JSX.Element;
    }
    return <span class="userinput-text">{s.props.querytext}</span>;
}

function InputBox({ s }: { s: UserInputState }): JSX.Element {
    if (s.props.responsetype === "confirm") {
        return <></>;
    }
    return (
        <TextInput
            variant="plain" type={s.props.publictext ? "text" : "password"}
            onInput={(e) => s.setResponseText((e.target as HTMLInputElement).value)}
            value={s.responseText()}
            maxLength={400}
            class="userinput-inputbox"
            autofocus={true}
            onKeyDown={(e) => keyutil.keydownWrapper((muxEvent) => onInputKeyDown(s, muxEvent))(e)}
        />
    );
}

function OptionalCheckbox({ s }: { s: UserInputState }): JSX.Element {
    if (s.props.checkboxmsg == "") {
        return <></>;
    }
    return (
        <div class="userinput-checkbox-container">
            <div class="userinput-checkbox-row">
                <input
                    type="checkbox"
                    id={`uicheckbox-${s.props.requestid}`}
                    class="userinput-checkbox"
                    ref={(el) => (s.checkbox.el = el)}
                />
                <label for={`uicheckbox-${s.props.requestid}`}>{s.props.checkboxmsg}</label>
            </div>
        </div>
    );
}

const UserInputModal = (props: UserInputProps) => {
    const s = createUserInputState(props);
    startCountdown(s);

    return (
        <Modal
            open={true}
            onClose={() => sendErrResponse(s)}
            size="md"
        >
            <ModalHeader title={`${props.title} (${s.countdown()}s)`} />
            <ModalBody>
                <div class="userinput-body">
                    <QueryText s={s} />
                    <InputBox s={s} />
                    <OptionalCheckbox s={s} />
                </div>
            </ModalBody>
            <ModalFooter>
                <Button onClick={() => negativeResponse(s)}>
                    {props.cancellabel ?? "Cancel"}
                </Button>
                <Button onClick={() => submit(s)}>
                    {props.oklabel ?? "Ok"}
                </Button>
            </ModalFooter>
        </Modal>
    );
};

UserInputModal.displayName = "UserInputModal";

export { UserInputModal };
