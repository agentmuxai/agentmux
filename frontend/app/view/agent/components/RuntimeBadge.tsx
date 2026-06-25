// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import "./RuntimeBadge.scss";

interface RuntimeBadgeProps {
    runtime: "host" | "container" | string;
    /** sm = card rows (10px); md = pane rows (11px). Default: sm. */
    size?: "sm" | "md";
}

export const RuntimeBadge = (props: RuntimeBadgeProps): JSX.Element => {
    const isContainer = () => props.runtime === "container";
    const isHost = () => props.runtime === "host";
    const isKnown = () => isContainer() || isHost();

    return (
        <span
            class={`runtime-badge runtime-badge--${isKnown() ? props.runtime : "unknown"} runtime-badge--${props.size ?? "sm"}`}
            title={isContainer() ? "Runs in an isolated Docker container" : isHost() ? "Runs directly on your machine with full system access" : props.runtime}
        >
            <i class={`fa-solid ${isContainer() ? "fa-box" : isHost() ? "fa-server" : "fa-circle-question"}`} aria-hidden="true" />
            {isContainer() ? "Container" : isHost() ? "Host" : props.runtime}
        </span>
    );
};
