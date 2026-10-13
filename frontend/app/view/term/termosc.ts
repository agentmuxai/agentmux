// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Terminal OSC escape sequence handlers.
// Extracted from termwrap.ts — pure functions, no TermWrap dependency.

import { authHeaders } from "@/app/store/auth-headers";
import { MOS, atoms } from "@/app/store/global";
import * as services from "@/app/store/services";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fireAndForget } from "@/util/util";
import { Terminal } from "@xterm/xterm";
import { handleAgentIdChange } from "./termagent";
import { setBlockMeta } from "@/app/store/block-meta";
import { META_OSC_TITLE } from "@/app/store/meta-keys";

// OSC 9283 — AgentMux meta commands
export function handleOscMuxCommand(data: string, blockId: string, loaded: boolean): boolean {
    if (!loaded) {
        return true;
    }
    if (!data || data.length === 0) {
        console.log("Invalid AgentMux OSC command received (empty)");
        return true;
    }

    // Expected format: "setmeta;{JSONDATA}", applied to this pane's own block. The JSON may
    // itself contain ";". Wave's "setmeta;[wave-id];{JSONDATA}" form, which targets another
    // object, isn't supported: it needed the `resolveids` RPC, which srv never had.
    const sep = data.indexOf(";");
    if (sep < 0 || data.slice(0, sep) !== "setmeta") {
        console.log("Invalid AgentMux OSC command received (bad command)", data);
        return true;
    }
    const jsonPayload = data.slice(sep + 1);
    if (!jsonPayload.trimStart().startsWith("{")) {
        console.log("AgentMux OSC setmeta with a target id isn't supported; it applies to its own pane only", data);
        return true;
    }

    let meta: any;
    try {
        meta = JSON.parse(jsonPayload);
    } catch (e) {
        console.error("Invalid JSON in AgentMux OSC command:", e);
        return true;
    }

    fireAndForget(() => {
        return services.ObjectService.UpdateObjectMeta(MOS.makeORef("block", blockId), meta);
    });
    return true;
}

// OSC 7 — Current working directory
// We return true always because we "own" OSC 7.
// Even if it is invalid we don't want to propagate to other handlers.
export function handleOsc7Command(data: string, blockId: string, loaded: boolean): boolean {
    if (!loaded) {
        return true;
    }
    if (data == null || data.length == 0) {
        console.log("Invalid OSC 7 command received (empty)");
        return true;
    }
    if (data.length > 1024) {
        console.log("Invalid OSC 7, data length too long", data.length);
        return true;
    }

    let pathPart: string;
    try {
        const url = new URL(data);
        if (url.protocol !== "file:") {
            console.log("Invalid OSC 7 command received (non-file protocol)", data);
            return true;
        }
        pathPart = decodeURIComponent(url.pathname);

        // Normalize double slashes at the beginning to single slash
        if (pathPart.startsWith("//")) {
            pathPart = pathPart.substring(1);
        }

        // Handle Windows paths (e.g., /C:/... or /D:\...)
        if (/^\/[a-zA-Z]:[\\/]/.test(pathPart)) {
            // Strip leading slash and normalize to forward slashes
            pathPart = pathPart.substring(1).replace(/\\/g, "/");
        }
    } catch (e) {
        console.log("Invalid OSC 7 command received (parse error)", data, e);
        return true;
    }

    setTimeout(() => {
        fireAndForget(async () => {
            await services.ObjectService.UpdateObjectMeta(MOS.makeORef("block", blockId), {
                "cmd:cwd": pathPart,
            });
        });
    }, 0);
    return true;
}

// OSC 0/2 — Window Title (used by Claude Code for activity summaries)
const titleUpdateDebounceMap = new Map<string, ReturnType<typeof setTimeout>>();
const TITLE_UPDATE_DEBOUNCE_MS = 300;

export function handleOscTitleCommand(data: string, blockId: string, loaded: boolean): boolean {
    if (!loaded) {
        return false; // Let xterm handle it too for window title
    }
    if (data == null || data.length === 0) {
        return false;
    }
    if (data.length > 256) {
        data = data.substring(0, 256);
    }

    let activity = data;
    if (activity.startsWith("Claude: ")) {
        activity = activity.substring(8);
    } else if (activity.startsWith("Claude Code: ")) {
        activity = activity.substring(13);
    }

    const existingTimeout = titleUpdateDebounceMap.get(blockId);
    if (existingTimeout) {
        clearTimeout(existingTimeout);
    }

    const timeout = setTimeout(() => {
        titleUpdateDebounceMap.delete(blockId);
        fireAndForget(async () => {
            await services.ObjectService.UpdateObjectMeta(MOS.makeORef("block", blockId), {
                [META_OSC_TITLE]: activity,
            } as any);
        });
    }, TITLE_UPDATE_DEBOUNCE_MS);

    titleUpdateDebounceMap.set(blockId, timeout);

    return false; // Return false to let xterm also handle it (for native window title)
}

// OSC 16162 — Shell Integration Commands
type Osc16162Command =
    | { command: "A"; data: {} }
    | { command: "C"; data: { cmd64?: string } }
    | { command: "M"; data: { shell?: string; shellversion?: string; uname?: string } }
    | { command: "D"; data: { exitcode?: number } }
    | { command: "I"; data: { inputempty?: boolean } }
    | { command: "R"; data: {} }
    | { command: "E"; data: { [key: string]: string } }
    | { command: "X"; data: { muxbus_url?: string; muxbus_token?: string } };

export function handleOsc16162Command(data: string, blockId: string, loaded: boolean, terminal: Terminal): boolean {
    if (!loaded) {
        return true;
    }
    if (!data || data.length === 0) {
        return true;
    }

    const parts = data.split(";");
    const commandStr = parts[0];
    const jsonDataStr = parts.length > 1 ? parts.slice(1).join(";") : null;
    let parsedData: Record<string, any> = {};
    if (jsonDataStr) {
        try {
            parsedData = JSON.parse(jsonDataStr);
        } catch (e) {
            console.error("Error parsing OSC 16162 JSON data:", e);
        }
    }

    const cmd: Osc16162Command = { command: commandStr, data: parsedData } as Osc16162Command;
    switch (cmd.command) {
        // A, C, M, D and I (prompt, command, shell, exit code, empty input) used to go to
        // setrtinfo, which srv never handled and nothing read; they're ignored until
        // something consumes them.
        case "R":
            if (terminal.buffer.active.type === "alternate") {
                terminal.write("\x1b[?1049l");
            }
            break;
        case "E":
            if (cmd.data && Object.keys(cmd.data).length > 0) {
                setTimeout(() => {
                    fireAndForget(async () => {
                        await setBlockMeta(blockId, { "cmd:env": cmd.data }).catch((e) => console.log("error setting cmd:env (OSC 16162 E)", e));
                    });
                }, 0);

                const agentId = cmd.data["AGENTMUX_AGENT_ID"] as string | undefined;
                const tabId = atoms.staticTabId();
                handleAgentIdChange(blockId, agentId, tabId);
            } else {
                // Empty payload: clear agent identity
                setTimeout(() => {
                    fireAndForget(async () => {
                        await setBlockMeta(blockId, { "cmd:env": null }).catch((e) => console.log("error clearing cmd:env", e));
                    });
                }, 0);
                handleAgentIdChange(blockId, undefined, atoms.staticTabId());
            }
            break;
        case "X":
            fireAndForget(async () => {
                try {
                    const url = getWebServerEndpoint() + "/agentmux/reactive/poller/config";
                    // X-AuthKey required after audit C1/C2 fix moved
                    // /agentmux/reactive/* under auth_middleware.
                    const headers = authHeaders({ "Content-Type": "application/json" });
                    const response = await fetch(url, {
                        method: "POST",
                        headers,
                        body: JSON.stringify({
                            muxbus_url: cmd.data.muxbus_url || "",
                            muxbus_token: cmd.data.muxbus_token || "",
                        }),
                    });
                    if (!response.ok) {
                        const data = await response.json();
                        console.error("[reactive] failed to configure agentmux:", data.error || response.status);
                    } else {
                        const data = await response.json();
                        console.log("[reactive] agentmux configured:", data.running ? "running" : "stopped");
                    }
                } catch (e) {
                    console.error("[reactive] error configuring agentmux:", e);
                }
            });
            break;
    }

    return true;
}
