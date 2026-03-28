// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Terminal OSC escape sequence handlers.
// Extracted from termwrap.ts — pure functions, no TermWrap dependency.

import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { WOS, atoms, globalStore } from "@/app/store/global";
import * as services from "@/app/store/services";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fireAndForget } from "@/util/util";
import { Terminal } from "@xterm/xterm";
import { handleAgentIdChange } from "./termagent";

// OSC 9283 — Wave meta commands
export function handleOscWaveCommand(data: string, blockId: string, loaded: boolean): boolean {
    if (!loaded) {
        return true;
    }
    if (!data || data.length === 0) {
        console.log("Invalid Wave OSC command received (empty)");
        return true;
    }

    // Expected formats:
    // "setmeta;{JSONDATA}"
    // "setmeta;[wave-id];{JSONDATA}"
    const parts = data.split(";");
    if (parts[0] !== "setmeta") {
        console.log("Invalid Wave OSC command received (bad command)", data);
        return true;
    }
    let jsonPayload: string;
    let waveId: string | undefined;
    if (parts.length === 2) {
        jsonPayload = parts[1];
    } else if (parts.length >= 3) {
        waveId = parts[1];
        jsonPayload = parts.slice(2).join(";");
    } else {
        console.log("Invalid Wave OSC command received (1 part)", data);
        return true;
    }

    let meta: any;
    try {
        meta = JSON.parse(jsonPayload);
    } catch (e) {
        console.error("Invalid JSON in Wave OSC command:", e);
        return true;
    }

    if (waveId) {
        fireAndForget(() => {
            return RpcApi.ResolveIdsCommand(TabRpcClient, { blockid: blockId, ids: [waveId] })
                .then((response: { resolvedids: { [key: string]: any } }) => {
                    const oref = response.resolvedids[waveId];
                    if (!oref) {
                        console.error("Failed to resolve wave id:", waveId);
                        return;
                    }
                    services.ObjectService.UpdateObjectMeta(oref, meta);
                })
                .catch((err: any) => {
                    console.error("Error resolving wave id", waveId, err);
                });
        });
    } else {
        fireAndForget(() => {
            return services.ObjectService.UpdateObjectMeta(WOS.makeORef("block", blockId), meta);
        });
    }
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
            await services.ObjectService.UpdateObjectMeta(WOS.makeORef("block", blockId), {
                "cmd:cwd": pathPart,
            });

            const rtInfo = { "cmd:hascurcwd": true };
            const rtInfoData: CommandSetRTInfoData = {
                oref: WOS.makeORef("block", blockId),
                data: rtInfo,
            };
            await RpcApi.SetRTInfoCommand(TabRpcClient, rtInfoData).catch((e) =>
                console.log("error setting RT info", e)
            );
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
            await services.ObjectService.UpdateObjectMeta(WOS.makeORef("block", blockId), {
                "term:activity": activity,
            } as any);
        });
    }, TITLE_UPDATE_DEBOUNCE_MS);

    titleUpdateDebounceMap.set(blockId, timeout);

    return false; // Return false to let xterm also handle it (for native window title)
}

// OSC 16162 — Shell Integration Commands
// See aiprompts/wave-osc-16162.md for full documentation
type Osc16162Command =
    | { command: "A"; data: {} }
    | { command: "C"; data: { cmd64?: string } }
    | { command: "M"; data: { shell?: string; shellversion?: string; uname?: string } }
    | { command: "D"; data: { exitcode?: number } }
    | { command: "I"; data: { inputempty?: boolean } }
    | { command: "R"; data: {} }
    | { command: "E"; data: { [key: string]: string } }
    | { command: "X"; data: { agentmux_url?: string; agentmux_token?: string } };

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
    const rtInfo: ObjRTInfo = {};
    switch (cmd.command) {
        case "A":
            rtInfo["shell:state"] = "ready";
            break;
        case "C":
            rtInfo["shell:state"] = "running-command";
            if (cmd.data.cmd64) {
                const decodedLen = Math.ceil(cmd.data.cmd64.length * 0.75);
                if (decodedLen > 8192) {
                    rtInfo["shell:lastcmd"] = `# command too large (${decodedLen} bytes)`;
                } else {
                    try {
                        const decodedCmd = atob(cmd.data.cmd64);
                        rtInfo["shell:lastcmd"] = decodedCmd;
                    } catch (e) {
                        console.error("Error decoding cmd64:", e);
                        rtInfo["shell:lastcmd"] = null;
                    }
                }
            } else {
                rtInfo["shell:lastcmd"] = null;
            }
            break;
        case "M":
            if (cmd.data.shell) {
                rtInfo["shell:type"] = cmd.data.shell;
            }
            if (cmd.data.shellversion) {
                rtInfo["shell:version"] = cmd.data.shellversion;
            }
            if (cmd.data.uname) {
                rtInfo["shell:uname"] = cmd.data.uname;
            }
            break;
        case "D":
            if (cmd.data.exitcode != null) {
                rtInfo["shell:lastcmdexitcode"] = cmd.data.exitcode;
            } else {
                rtInfo["shell:lastcmdexitcode"] = null;
            }
            break;
        case "I":
            if (cmd.data.inputempty != null) {
                rtInfo["shell:inputempty"] = cmd.data.inputempty;
            }
            break;
        case "R":
            if (terminal.buffer.active.type === "alternate") {
                terminal.write("\x1b[?1049l");
            }
            break;
        case "E":
            if (cmd.data && Object.keys(cmd.data).length > 0) {
                setTimeout(() => {
                    fireAndForget(async () => {
                        await RpcApi.SetMetaCommand(TabRpcClient, {
                            oref: WOS.makeORef("block", blockId),
                            meta: { "cmd:env": cmd.data },
                        }).catch((e) => console.log("error setting cmd:env (OSC 16162 E)", e));
                    });
                }, 0);

                const agentId = cmd.data["AGENTMUX_AGENT_ID"] as string | undefined;
                const tabId = atoms.staticTabId();
                handleAgentIdChange(blockId, agentId, tabId);
            } else {
                // Empty payload: clear agent identity
                setTimeout(() => {
                    fireAndForget(async () => {
                        await RpcApi.SetMetaCommand(TabRpcClient, {
                            oref: WOS.makeORef("block", blockId),
                            meta: { "cmd:env": null },
                        }).catch((e) => console.log("error clearing cmd:env", e));
                    });
                }, 0);
                handleAgentIdChange(blockId, undefined, atoms.staticTabId());
            }
            break;
        case "X":
            fireAndForget(async () => {
                try {
                    const url = getWebServerEndpoint() + "/wave/reactive/poller/config";
                    const response = await fetch(url, {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: JSON.stringify({
                            agentmux_url: cmd.data.agentmux_url || "",
                            agentmux_token: cmd.data.agentmux_token || "",
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

    if (Object.keys(rtInfo).length > 0) {
        setTimeout(() => {
            fireAndForget(async () => {
                const rtInfoData: CommandSetRTInfoData = {
                    oref: WOS.makeORef("block", blockId),
                    data: rtInfo,
                };
                await RpcApi.SetRTInfoCommand(TabRpcClient, rtInfoData).catch((e) =>
                    console.log("error setting RT info (OSC 16162)", e)
                );
            });
        }, 0);
    }

    return true;
}
