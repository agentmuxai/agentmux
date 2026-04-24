// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { Markdown } from "@/element/markdown";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/element/modal-v2";
import { modalsModel } from "@/store/modalmodel";
import * as keyutil from "@/util/keyutil";
import { fireAndForget } from "@/util/util";
import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { UserInputService } from "../store/services";
import "./userinputmodal.scss";

const UserInputModal = (userInputRequest: UserInputRequest) => {
    const [responseText, setResponseText] = createSignal("");
    const [countdown, setCountdown] = createSignal(Math.floor(userInputRequest.timeoutms / 1000));
    let checkboxRef!: HTMLInputElement;

    const handleSendErrResponse = () => {
        fireAndForget(() =>
            UserInputService.SendUserInputResponse({
                type: "userinputresp",
                requestid: userInputRequest.requestid,
                errormsg: "Canceled by the user",
            })
        );
        modalsModel.popModal();
    };

    const handleSendText = () => {
        fireAndForget(() =>
            UserInputService.SendUserInputResponse({
                type: "userinputresp",
                requestid: userInputRequest.requestid,
                text: responseText(),
                checkboxstat: checkboxRef?.checked ?? false,
            })
        );
        modalsModel.popModal();
    };

    const handleSendConfirm = (response: boolean) => {
        fireAndForget(() =>
            UserInputService.SendUserInputResponse({
                type: "userinputresp",
                requestid: userInputRequest.requestid,
                confirm: response,
                checkboxstat: checkboxRef?.checked ?? false,
            })
        );
        modalsModel.popModal();
    };

    const handleSubmit = () => {
        switch (userInputRequest.responsetype) {
            case "text":
                handleSendText();
                break;
            case "confirm":
                handleSendConfirm(true);
                break;
        }
    };

    const handleKeyDown = (waveEvent: WaveKeyboardEvent): boolean => {
        // Enter submits; ESC is handled by modal-v2's closeOnEscape
        // which calls our onClose → handleSendErrResponse.
        if (keyutil.checkKeyPressed(waveEvent, "Enter")) {
            handleSubmit();
            return true;
        }
    };

    // Countdown timer using setInterval — fires an err response when it
    // hits zero. Cleanup on unmount so a cancel/submit stops the tick.
    let intervalId: ReturnType<typeof setInterval>;
    intervalId = setInterval(() => {
        setCountdown((prev) => {
            if (prev <= 1) {
                clearInterval(intervalId);
                setTimeout(() => handleSendErrResponse(), 300);
                return 0;
            }
            return prev - 1;
        });
    }, 1000);
    onCleanup(() => clearInterval(intervalId));

    const queryText = (): JSX.Element => {
        if (userInputRequest.markdown) {
            return <Markdown text={userInputRequest.querytext} class="userinput-markdown" /> as JSX.Element;
        }
        return <span class="userinput-text">{userInputRequest.querytext}</span>;
    };

    const inputBox = (): JSX.Element => {
        if (userInputRequest.responsetype === "confirm") {
            return <></>;
        }
        return (
            <input
                type={userInputRequest.publictext ? "text" : "password"}
                onInput={(e) => setResponseText((e.target as HTMLInputElement).value)}
                value={responseText()}
                maxLength={400}
                class="userinput-inputbox"
                autofocus={true}
                onKeyDown={(e) => keyutil.keydownWrapper(handleKeyDown)(e)}
            />
        );
    };

    const optionalCheckbox = (): JSX.Element => {
        if (userInputRequest.checkboxmsg == "") {
            return <></>;
        }
        return (
            <div class="userinput-checkbox-container">
                <div class="userinput-checkbox-row">
                    <input
                        type="checkbox"
                        id={`uicheckbox-${userInputRequest.requestid}`}
                        class="userinput-checkbox"
                        ref={checkboxRef}
                    />
                    <label for={`uicheckbox-${userInputRequest.requestid}`}>{userInputRequest.checkboxmsg}</label>
                </div>
            </div>
        );
    };

    const handleNegativeResponse = () => {
        switch (userInputRequest.responsetype) {
            case "text":
                handleSendErrResponse();
                break;
            case "confirm":
                handleSendConfirm(false);
                break;
        }
    };

    return (
        <Modal
            open={true}
            onClose={handleSendErrResponse}
            size="md"
        >
            <ModalHeader title={`${userInputRequest.title} (${countdown()}s)`} />
            <ModalBody>
                <div class="userinput-body">
                    {queryText()}
                    {inputBox()}
                    {optionalCheckbox()}
                </div>
            </ModalBody>
            <ModalFooter>
                <Button onClick={handleNegativeResponse}>
                    {userInputRequest.cancellabel ?? "Cancel"}
                </Button>
                <Button onClick={handleSubmit}>
                    {userInputRequest.oklabel ?? "Ok"}
                </Button>
            </ModalFooter>
        </Modal>
    );
};

UserInputModal.displayName = "UserInputModal";

export { UserInputModal };
