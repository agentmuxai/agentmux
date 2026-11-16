// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the person is told when `agent.account.switch` answers: nothing for a switch that happened cleanly.

export type SwitchNotice = ["info" | "warning", title: string, message: string];

export function switchNotice(r: { status: string; failed: string[] }, label: string): SwitchNotice | undefined {
    if (r.status === "unchanged") return ["info", `Staying on ${label}`, "The queued switch was cancelled."];
    if (r.status === "preparing") {
        return ["info", `Switching to ${label}`, "The agent is reaching a safe point first; the chip counts down. Click it to switch now."];
    }
    if (r.status === "queued") {
        return ["info", `Switching to ${label} after this turn`, "The agent keeps working; it moves when the turn ends."];
    }
    if (r.failed.length > 0) return ["warning", `Switched to ${label}, with a problem`, r.failed.join(" ")];
    return undefined;
}
