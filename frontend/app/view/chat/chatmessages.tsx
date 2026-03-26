// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Markdown } from "@/app/element/markdown";
import clsx from "clsx";
import { For } from "solid-js";
import type { JSX } from "solid-js";

import "./chatmessages.scss";

export interface ChatMessage {
    id: string;
    username: string;
    message: string;
    color?: string;
    userIcon?: string;
}

interface ChatMessagesProps {
    messages: ChatMessage[];
    class?: string;
}

function ChatMessages(props: ChatMessagesProps): JSX.Element {
    let messagesEndRef!: HTMLDivElement;

    return (
        <div class={clsx("chat-messages overflow-y-auto", props.class)}>
            <For each={props.messages}>
                {({ id, username, message, color, userIcon }) => (
                    <div class="chat-message">
                        {userIcon && <img src={userIcon} alt="user icon" class="chat-user-icon" />}
                        <span class="chat-username" style={{ color: color || "var(--main-text-color)" }}>
                            {username}:
                        </span>
                        <span class="chat-text">
                            <Markdown scrollable={false} text={message} />
                        </span>
                    </div>
                )}
            </For>
            <div ref={messagesEndRef!} />
        </div>
    );
}

export { ChatMessages };
