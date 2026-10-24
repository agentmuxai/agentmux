// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import "./RuntimeBadge.scss";

interface RuntimeBadgeProps {
    runtime: "host" | "container" | string;
    /** sm = card rows (10px); md = pane rows (11px); tag = minimal
     *  icon-less text label (9px, NATIVE/CONTAINER wording) for the agent
     *  composer strip, next to the model selector — see
     *  docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md §3.2.
     *  Default: sm. */
    size?: "sm" | "md" | "tag";
}

export const RuntimeBadge = (props: RuntimeBadgeProps): JSX.Element => {
    const isContainer = () => props.runtime === "container";
    const isHost = () => props.runtime === "host";
    const isKnown = () => isContainer() || isHost();
    const isTag = () => props.size === "tag";

    // The composer-strip "tag" variant intentionally uses different wording
    // (NATIVE/CONTAINER, all-caps) than the sm/md badge (Native/Container) — a
    // deliberate, explicit design choice for that specific compact spot,
    // not an oversight. Both read from the same underlying runtime value.
    const label = (): string => {
        if (isTag()) return isContainer() ? "CONTAINER" : isHost() ? "NATIVE" : props.runtime;
        return isContainer() ? "Container" : isHost() ? "Native" : props.runtime;
    };

    return (
        <span
            class={`runtime-badge runtime-badge--${isKnown() ? props.runtime : "unknown"} runtime-badge--${props.size ?? "sm"}`}
            title={isContainer() ? "Container (runs in Docker, isolated)" : isHost() ? "Native (runs on this computer, with full access)" : props.runtime}
        >
            <Show when={!isTag()}>
                <i class={`fa-solid ${isContainer() ? "fa-box" : isHost() ? "fa-server" : "fa-circle-question"}`} aria-hidden="true" />
            </Show>
            {label()}
        </span>
    );
};
