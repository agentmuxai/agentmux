// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { JSX, Show } from "solid-js";
import "./avatar.scss";

interface AvatarProps {
    name: string;
    status: "online" | "offline" | "busy" | "away";
    className?: string;
    imageUrl?: string;
}

const Avatar = ({ name, status = "offline", className, imageUrl }: AvatarProps): JSX.Element => {
    const getInitials = (name: string) => {
        const nameParts = name.split(" ");
        const initials = nameParts.map((part) => part[0]).join("");
        return initials.toUpperCase();
    };

    return (
        <div class={clsx("avatar", status, className)} title="status">
            <Show
                when={imageUrl}
                fallback={<div class="avatar-initials">{getInitials(name)}</div>}
            >
                <img src={imageUrl} alt={`${name}'s avatar`} class="avatar-image" />
            </Show>
            <div class={`status-indicator ${status}`} />
        </div>
    );
};

export { Avatar };
