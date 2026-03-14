// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { EmojiPalette, type EmojiItem } from "@/app/element/emojipalette";
import { InputGroup } from "@/app/element/input";
import { MultiLineInput } from "@/app/element/multilineinput";
import * as keyutil from "@/util/keyutil";
import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { throttle } from "throttle-debounce";

interface ChatBoxProps {
    onSendMessage: (message: string) => void;
}

function ChatBox(props: ChatBoxProps): JSX.Element {
    const [message, setMessage] = createSignal("");
    let multiLineInputRef!: HTMLTextAreaElement;

    const handleInputChange = (e: Event) => {
        setMessage((e.target as HTMLTextAreaElement).value);
    };

    const handleKeyDown = (waveEvent: WaveKeyboardEvent): boolean => {
        if (keyutil.checkKeyPressed(waveEvent, "Enter") && !waveEvent.shift && message().trim() !== "") {
            props.onSendMessage(message());
            setMessage("");
            return true;
        }
        return false;
    };

    const handleEmojiSelect = (emojiItem: EmojiItem) => {
        if (multiLineInputRef) {
            const { selectionStart, selectionEnd } = multiLineInputRef;
            const currentValue = multiLineInputRef.value;
            const newValue =
                currentValue.substring(0, selectionStart) + emojiItem.emoji + currentValue.substring(selectionEnd);

            setMessage(newValue);
            multiLineInputRef.value = newValue;

            const cursorPosition = selectionStart + emojiItem.emoji.length;
            throttle(0, () => {
                if (multiLineInputRef) {
                    multiLineInputRef.selectionStart = multiLineInputRef.selectionEnd = cursorPosition;
                    multiLineInputRef.focus();
                }
            })();

            multiLineInputRef.dispatchEvent(new Event("change", { bubbles: true }));
        }
    };

    return (
        <InputGroup className="chatbox">
            <MultiLineInput
                ref={multiLineInputRef!}
                className="input"
                value={message()}
                onChange={handleInputChange}
                onKeyDown={(e: KeyboardEvent) => keyutil.keydownWrapper(handleKeyDown)(e)}
                placeholder="Type a message..."
            />
            <EmojiPalette placement="top-end" onSelect={handleEmojiSelect} />
        </InputGroup>
    );
}

export { ChatBox };
