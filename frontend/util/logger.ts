// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/app/store/global";

type LogData = Record<string, any>;

function log(level: string, module: string, message: string, data?: LogData) {
    const consoleFn =
        level === "error" ? console.error : level === "warn" ? console.warn : console.log;
    consoleFn(`[${module}] ${message}`, data ?? "");

    try {
        getApi().sendLogStructured(level, module, message, data ?? null);
    } catch {
        // Silently ignore if Tauri bridge not ready
    }
}

export const Logger = {
    error: (module: string, msg: string, data?: LogData) => log("error", module, msg, data),
    warn: (module: string, msg: string, data?: LogData) => log("warn", module, msg, data),
    info: (module: string, msg: string, data?: LogData) => log("info", module, msg, data),
    debug: (module: string, msg: string, data?: LogData) => log("debug", module, msg, data),
};
