// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Modal } from "@/app/modals/modal";
import { Markdown } from "@/element/markdown";
import { modalsModel } from "@/store/modalmodel";
import * as keyutil from "@/util/keyutil";
import { fireAndForget } from "@/util/util";
import { createSignal, Show, type JSX } from "solid-js";
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
        if (keyutil.checkKeyPressed(waveEvent, "Escape")) {
            handleSendErrResponse();
            return;
        }
        if (keyutil.checkKeyPressed(waveEvent, "Enter")) {
            handleSubmit();
            return true;
        }
    };

    // Countdown timer using setInterval
    let intervalId: ReturnType<typeof setInterval>;
    const startCountdown = () => {
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
    };
    startCountdown();

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
            onOk={() => handleSubmit()}
            onCancel={() => handleNegativeResponse()}
            onClose={() => handleSendErrResponse()}
            okLabel={userInputRequest.oklabel}
            cancelLabel={userInputRequest.cancellabel}
        >
            <div class="userinput-header">{userInputRequest.title + ` (${countdown()}s)`}</div>
            <div class="userinput-body">
                {queryText()}
                {inputBox()}
                {optionalCheckbox()}
            </div>
        </Modal>
    );
};

UserInputModal.displayName = "UserInputModal";

export { UserInputModal };
