// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useLaunchLogs — terminal-style log line accumulator used during the
 * agent launch flow.
 *
 * Step 3 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Returns:
 *   - `lines` — reactive accessor for the current LogLine[]
 *   - `append` — add one line. Matches the `LogFn` signature used by
 *     `runLaunchFlow` in flows/launch-flow.ts, so the caller can pass
 *     `append` directly as the `log` option.
 *   - `clear` — reset the log buffer (used on retry)
 *
 * No RPCs, no subscriptions, no side effects — this is a pure
 * createSignal wrapper. The only reason it's a hook at all is so the
 * three log-management pieces (state, append, clear) live together
 * instead of being scattered across agent-view.tsx.
 */

import { createSignal, type Accessor } from "solid-js";
import type { LogLine } from "../types";

export interface UseLaunchLogs {
    lines: Accessor<LogLine[]>;
    append: (tag: string, text: string, level?: "info" | "error" | "warn") => void;
    clear: () => void;
}

export function useLaunchLogs(): UseLaunchLogs {
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
