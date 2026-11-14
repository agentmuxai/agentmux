// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's runtime state as a chip: in My Agents and in the Swarm's stopped agents, so one state
// reads the same everywhere (store/agent-runtime.ts has the states).

import { Pill, type PillTone } from "@/app/element/ui";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { JSX } from "solid-js";

export interface RuntimeChip {
    label: string;
    tone: PillTone;
    icon?: string;
}

const CHIPS: Record<AgentRuntime["state"], RuntimeChip> = {
    stopped: { label: "stopped", tone: "neutral" },
    starting: { label: "starting", tone: "neutral", icon: "circle-notch" },
    idle: { label: "idle", tone: "ok" },
    working: { label: "working", tone: "error" }, // red, as in the Swarm
    waiting_on_user: { label: "waiting on you", tone: "warn", icon: "circle-question" },
    limited: { label: "limited", tone: "warn" },
    signed_out: { label: "signed out", tone: "warn" },
    error: { label: "error", tone: "error" },
};

/** What srv's reason codes (`AgentRuntime.detail`) say, for the chip's tooltip. */
const DETAIL_TEXT: Record<string, string> = {
    usage_limit: "Out of usage until it resets",
    needs_login: "Signed out: sign in to continue",
    starts_on_first_message: "Runs on its first message",
    browser_request: "Waiting for your answer in a browser pane",
};

/** A stopped agent that is wanted running but cannot start: not the same as one stopped on purpose. */
const BLOCKED_CHIP: RuntimeChip = { label: "can't start", tone: "warn", icon: "triangle-exclamation" };

export function runtimeChip(state: AgentRuntime["state"], blocked?: string): RuntimeChip {
    return blocked && state === "stopped" ? BLOCKED_CHIP : CHIPS[state];
}

/** Reads `props.state`, not a destructured `state`: the chip follows the agent from working to idle. */
export function AgentRuntimeChip(props: { state: AgentRuntime["state"]; detail?: string; blocked?: string }): JSX.Element {
    const chip = () => runtimeChip(props.state, props.blocked);
    const title = () => {
        if (props.blocked && props.state === "stopped") return `Wanted running, but it cannot start: ${props.blocked}`;
        return props.detail ? (DETAIL_TEXT[props.detail] ?? props.detail) : undefined;
    };
    return (
        <Pill
            tone={chip().tone}
            icon={chip().icon}
            title={title()}
            class={`agent-runtime-chip agent-runtime-chip--${props.state}`}
        >
            {chip().label}
        </Pill>
    );
}
