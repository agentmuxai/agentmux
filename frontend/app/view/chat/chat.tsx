// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { ChatMessage, ChatMessages } from "@/app/view/chat/chatmessages";
import { UserStatus } from "@/app/view/chat/userlist";
import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { Channels } from "./channels";
import { ChatBox } from "./chatbox";
import { channels, messages, users } from "./data";
import { UserList } from "./userlist";

import "./chat.scss";

class ChatModel {
    viewType: string;
    channels: MenuItem[];
    users: UserStatus[];

    private _messages: () => ChatMessage[];
    private _setMessages: (v: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void;

    constructor(blockId: string) {
        this.viewType = "chat";
        this.channels = channels;
        this.users = users;
        const [msgs, setMsgs] = createSignal<ChatMessage[]>(messages);
        this._messages = msgs;
        this._setMessages = setMsgs;
    }

    getMessages(): ChatMessage[] {
        return this._messages();
    }

    addMessage(newMessage: ChatMessage) {
        this._setMessages((prev) => [...prev, newMessage]);
    }
}

interface ChatProps {
    model: ChatModel;
}

function Chat(props: ChatProps): JSX.Element {
    const { channels, users } = props.model;

    const handleSendMessage = (message: string) => {
        const newMessage: ChatMessage = {
            id: `${Date.now()}`,
            username: "currentUser",
            message: message,
        };
        props.model.addMessage(newMessage);
    };

    return (
        <div class="chat-view">
            <Channels channels={channels} />
            <div class="chat-section">
                <div class="message-wrapper">
                    <ChatMessages messages={props.model.getMessages()} />
                </div>
                <ChatBox onSendMessage={(message: string) => handleSendMessage(message)} />
            </div>
            <UserList users={users} />
        </div>
    );
}

export { Chat, ChatModel };
