// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { For } from "solid-js";
import type { JSX } from "solid-js";
import { Avatar } from "../../element/avatar";
import "./userlist.scss";

export interface UserStatus {
    label: string;
    status: "online" | "busy" | "away" | "offline";
    onClick: () => void;
    avatarUrl?: string;
}

interface UserListProps {
    users: UserStatus[];
    class?: string;
}

function UserList(props: UserListProps): JSX.Element {
    return (
        <div class={clsx("user-list", props.class)}>
            <For each={props.users}>
                {({ label, status, onClick, avatarUrl }) => (
                    <div class={clsx("user-status-item", status)} onClick={onClick}>
                        <div class="user-status-icon">
                            <Avatar name={label} status={status} className="size-sm" imageUrl={avatarUrl} />
                        </div>
                        <div class="user-status-text">{label}</div>
                    </div>
                )}
            </For>
        </div>
    );
}

export { UserList };
