// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// WPS event-name constants — mirrors agentmux-srv/src/backend/wps.rs:22-54.
// Use these instead of bare string literals so typos are caught at build time
// and grepping for an event name finds all its usages in one search.

export const WpsEvent = {
    BlockFile: "blockfile",
    BlockClose: "blockclose",
    ConnChange: "connchange",
    SysInfo: "sysinfo",
    ControllerStatus: "controllerstatus",
    WaveObjUpdate: "waveobj:update",
    InstallProgress: "install_progress",
    Config: "config",
    UserInput: "userinput",
    AgentMessageAccepted: "agent-message-accepted",
    RouteGone: "route:gone",
    BlockStats: "blockstats",
    AgentHealth: "agenthealth",
    AgentFailure: "agentfailure",
    ShellNodeCreate: "shell_node_create",
    ShellChunk: "shell_chunk",
    BlockActivity: "block:activity",
} as const;

export type WpsEventName = (typeof WpsEvent)[keyof typeof WpsEvent];
