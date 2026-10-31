// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Once a turn ends, tool calls still marked running are orphans. A foreground call cannot outlive
 * its turn (it blocks it), and a backgrounded one resolves its ToolNode at once, so a call still
 * `running` shortly after `TurnEnd` is a rejected call or a dropped result. It showed as a "running
 * 45m" dock row for the rest of the session; the scrub otherwise runs only at session boundaries.
 *
 * The 2s delay lets a tail flush land. A new turn already started skips the pass: its running tools
 * are live. Tools only: thinking markdown, shells (independent of the turn) and questions keep
 * their session-bounded lifecycles.
 */

import { createEffect, on, onCleanup, type Accessor } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";

export const ORPHAN_SCRUB_DELAY_MS = 2_000;

/** `turnEnded` counts turn ends (it is 0 until the first). */
export function useTurnEndSweep(opts: { turnEnded: Accessor<number>; paneModel: AgentPaneModel; onTurnEnded?: () => void }): void {
    createEffect(
        on(opts.turnEnded, (n) => {
            if (n === 0) return;
            opts.onTurnEnded?.();
            const timer = setTimeout(() => {
                if (workingFromPhase(opts.paneModel.state.turnPhase)) return;
                opts.paneModel.dispatchDoc({ type: "ScrubOrphanedInProgress", at: Date.now(), scope: "tools-only" });
            }, ORPHAN_SCRUB_DELAY_MS);
            onCleanup(() => clearTimeout(timer));
        }),
    );
}
