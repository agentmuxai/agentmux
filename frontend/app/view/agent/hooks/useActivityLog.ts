// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useActivityLog — tagged log of per-pane activity. Accumulates entries
 * from the whole session lifecycle, not just startup:
 *
 *   - Launch flow (cli, docker, auth, install)
 *   - Subprocess lifecycle (`[subprocess] spawned`, `turn complete`, exit codes)
 *   - Slash command outcomes
 *   - History pagination, subagent events, bookmark save failures
 *   - Errors routed via `log("error", …, "error")`
 *
 * Rendered in `<ActivityLogPanel>` as a collapsible pane docked above
 * the composer. Used to be called `useLaunchLogs` and lived at the top
 * of the conversation scroll area; renamed + relocated in the
 * activity-log-panel PR. See
 * `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 *
 * Returns:
 *   - `lines` — reactive accessor for the current `LogLine[]` (FIFO)
 *   - `append` — add one entry. Matches the `LogFn` signature used by
 *     `runLaunchFlow` and every hook that takes an `opts.log` — no
 *     call-site signature change.
 *   - `clear` — reset the buffer (used on /clear slash command + retry)
 *
 * No RPCs, no subscriptions. Just a signal wrapper; the hook exists so
 * the three pieces (state, append, clear) live together.
 */

import { createSignal, type Accessor } from "solid-js";
import type { LogLine } from "../types";

export interface UseActivityLog {
    lines: Accessor<LogLine[]>;
    append: (tag: string, text: string, level?: "info" | "error" | "warn") => void;
    clear: () => void;
}

export function useActivityLog(): UseActivityLog {
    const [lines, setLines] = createSignal<LogLine[]>([]);
    let nextId = 1;

    const append = (tag: string, text: string, level?: "info" | "error" | "warn") => {
        setLines((prev) => [
            ...prev,
            {
                id: `log-${nextId++}`,
                tag,
                text,
                level: level ?? "info",
                timestamp: Date.now(),
            },
        ]);
    };

    const clear = () => setLines([]);

    return { lines, append, clear };
}
