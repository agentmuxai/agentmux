// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import "./RuntimeBadge.scss";

interface RuntimeBadgeProps {
    runtime: "host" | "container" | string;
    /** sm = card rows (10px); md = pane rows (11px). Default: sm. */
    size?: "sm" | "md";
}

export const RuntimeBadge = (props: RuntimeBadgeProps): JSX.Element | null => {
    if (props.runtime !== "host" && props.runtime !== "container") return null;

    const isContainer = () => props.runtime === "container";

    return (
        <span
            class={`runtime-badge runtime-badge--${props.runtime} runtime-badge--${props.size ?? "sm"}`}
            title={isContainer() ? "Runs in an isolated Docker container" : "Runs directly on your machine with full system access"}
        >
            <i class={`fa-solid ${isContainer() ? "fa-box" : "fa-server"}`} aria-hidden="true" />
            {isContainer() ? "Container" : "Host"}
        </span>
    );
};
