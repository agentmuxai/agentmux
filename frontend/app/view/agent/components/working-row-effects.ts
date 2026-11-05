// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's three small effects while it is live: the rotating phrase, the load start, and the phrase the worked line borrows. */

import { createEffect, onCleanup, type Accessor } from "solid-js";

import { pickThinkingPhrase } from "./status-formatters";
import type { RowState } from "./working-row-state";

export function useRowLiveEffects(s: RowState, live: Accessor<boolean>) {
    createEffect(() => {
        if (!live()) return;
        s.setPhrase(pickThinkingPhrase());
        const id = setInterval(() => {
            s.setPhrase((prev) => {
                const next = pickThinkingPhrase(prev);
                s.setLastPhrase(next);
                return next;
            });
        }, 30000);
        onCleanup(() => clearInterval(id));
    });

    createEffect(() => {
        if (live()) {
            s.setLoadStartMs((prev) => prev ?? Date.now());
        } else {
            s.setLoadStartMs(null);
        }
    });

    createEffect(() => {
        if (live()) s.setLastPhrase(s.phrase());
    });
}
