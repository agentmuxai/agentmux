// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A handler in a pane's own reactive scope, so a throw there is logged with the pane and the part of the pane
 * that threw. Without one, what reached the window's root handler (containBindingErrors) named no pane, and a
 * computation with no handler above it at all aborted the flush silently: the 2026-11-18 window whose panes stopped
 * following the layout logged nothing that said which pane.
 *
 * Two modes, by what sits above the scope:
 * - `pass-on` (default): log, then hand the error to the handler above, so a boundary that shows a fallback
 *   (BlockErrorBoundary) still takes it. Solid runs a handler's own throw through the next owner's handler.
 * - `contain`: log, and end it there. For a scope with nothing above it (a view model's own root, which has no
 *   owner). A throw while the scope is being built still reaches the code that builds it, in either mode.
 */

import { catchError } from "solid-js";
import { getApi } from "@/util/app-api";
import { resolveStackSync } from "@/log/source-map-resolver";

export type PaneErrorMode = "pass-on" | "contain";

/** An error already logged for a pane, so one passed up through nested scopes is written once, by the nearest. */
const reported = new WeakSet<object>();

/** Logs `error` as thrown in `scope` of the pane `blockId`. Never throws. Returns false when it was already logged. */
export function reportPaneError(blockId: string, scope: string, error: unknown): boolean {
    try {
        if (typeof error === "object" && error !== null) {
            if (reported.has(error)) return false;
            reported.add(error);
        }
        const err = error instanceof Error ? error : null;
        const name = err?.name ?? "Error";
        const message = err?.message ?? String(error);
        const raw = err?.stack ?? null;
        let stack = raw;
        try {
            if (raw) stack = resolveStackSync(raw).resolved;
        } catch {
            stack = raw;
        }
        getApi().sendLogStructured(
            "error",
            "pane-scope",
            `[pane-scope] ${name}: ${message} (block=${blockId.substring(0, 7)}, in=${scope})`,
            { block_id: blockId, scope, error_name: name, error_message: message, error_stack: stack, error_stack_raw: raw }
        );
    } catch {
        // Logging must never add a fault to the one it reports.
    }
    return true;
}

/** Builds `build` under a handler that logs a throw in it with the pane and `scope`; see the file's note for `mode`. */
export function catchPaneErrors<T>(blockId: string, scope: string, build: () => T, mode: PaneErrorMode = "pass-on"): T {
    // A throw in `build` itself is taken here, not left to Solid: inside a flush Solid queues the handler to run
    // later, when the caller has already gone on. It is logged and thrown to the caller, as without a scope.
    let failed: { error: unknown } | null = null;
    const built = catchError(
        () => {
            try {
                return build();
            } catch (error) {
                failed = { error };
                return undefined as T;
            }
        },
        (error) => {
            reportPaneError(blockId, scope, error);
            if (mode === "pass-on") throw error;
        }
    );
    if (failed) {
        reportPaneError(blockId, scope, (failed as { error: unknown }).error);
        throw (failed as { error: unknown }).error;
    }
    return built as T;
}
