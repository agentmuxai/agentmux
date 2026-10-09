// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The desktop (CEF) host's startup hooks: the default `@host-module`
// (app/host/host-module.ts).

import type { HostModule } from "@/app/host/host-module";
import { invokeCommand } from "@/app/platform/ipc";
import { setupCefApi } from "./cef-init";
import { initErrorForwarder } from "./log/error-forwarder";
import { initLogPipe } from "./log/log-pipe";

export const hostModule: HostModule = {
    early() {
        // Pipe all console.log/warn/error to the Rust host log file, first, so
        // early messages are captured.
        initLogPipe();
        // Capture uncaught errors + unhandled promise rejections and forward
        // them via the same fe_log_structured IPC channel as the console pipe.
        // SolidJS reconciler DOM exceptions (e.g. replaceChild NotFoundError)
        // surface only as window "error" events and would otherwise leave no
        // trace in the host log. Retro 2026-05-23 (agent-pane cascade →
        // replaceChild quick-win).
        initErrorForwarder();
    },

    // ── First-paint signal (Linux startup white-flash fix) ──────────────────
    // docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md.
    //
    // Tell the host the moment the browser has actually composited a frame —
    // not "main-frame load complete" (CEF's `on_load_end`, which can fire
    // before anything has visually painted and is what the host used to gate
    // the native window's show() on).
    //
    // Fired directly via invokeCommand() rather than getApi() — getApi() isn't
    // installed until setupCefApi()'s full IPC batch resolves, which can take
    // seconds on a slow start (see the spec's profiling numbers) and would be
    // far too late to gate the window's first show(). invokeCommand() only
    // needs `window.__AGENTMUX_IPC_PORT__`/`__AGENTMUX_IPC_TOKEN__`, which on
    // a normal (non-reload) launch are already present from the boot URL's
    // query params — no wait required. Harmless no-op outside CEF
    // (invokeCommand rejects; caught and ignored) and on platforms that don't
    // gate on this signal (Windows/macOS currently just log it).
    firstPaint(label) {
        invokeCommand("report_first_paint", { label }).catch(() => {});
    },

    setup: setupCefApi,
};
