// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { fireAndForget } from "@/util/util";

// Debug logging function - writes to file
const DEBUG_LOG_PATH = "C:/Systems/agentmux-debug.log";

function stringToBase64(str: string): string {
    const bytes = new TextEncoder().encode(str);
    let binary = "";
    for (let i = 0; i < bytes.length; i++) {
        binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
}

export function debugLog(message: string, data?: unknown): void {
    const timestamp = new Date().toISOString();
    const logLine = `[${timestamp}] [KEYMODEL] ${message}${data !== undefined ? ": " + JSON.stringify(data) : ""}\n`;
    fireAndForget(async () => {
        try {
            await RpcApi.FileAppendCommand(TabRpcClient, {
                info: { path: DEBUG_LOG_PATH },
                data64: stringToBase64(logLine),
            });
        } catch (e) {
            console.error("Failed to write debug log:", e);
        }
    });
}
