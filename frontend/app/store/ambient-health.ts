// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How AgentMux's own model calls (titles, names, suggestions, narration, summaries, Supervisor
// verdicts) have gone since its server started, shared by the token indicator (a dot when a purpose
// is failing) and the token popover (the per-purpose counts). The counts come from
// `AmbientOutcomesCommand`; the summary logic is `ambient-outcomes.ts`.
// docs/specs/SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §6.

import { createSignal } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { summarizeAmbientOutcomes, type AmbientOutcomeSummary, type AmbientOutcomes } from "./ambient-outcomes";

/** How often the indicator asks while the popover is closed. */
export const AMBIENT_BACKGROUND_MS = 60_000;
/** How often the open popover asks. */
export const AMBIENT_OPEN_MS = 5_000;

const [outcomes, setOutcomes] = createSignal<AmbientOutcomes | null>(null);
const [failed, setFailed] = createSignal(false);

/** `null` until some call has ended at least once. */
export const ambientSummary = (): AmbientOutcomeSummary | null => summarizeAmbientOutcomes(outcomes());
/** How many purposes are failing. */
export const ambientFailingCount = (): number => ambientSummary()?.unhealthyCount ?? 0;
/** The last ask failed. The counts stay: a failed ask says nothing about them. */
export const ambientAskFailed = failed;

export async function refreshAmbient(): Promise<void> {
    try {
        setOutcomes((await RpcApi.AmbientOutcomesCommand(TabRpcClient, {})) ?? null);
        setFailed(false);
    } catch {
        setFailed(true);
    }
}

/** Ask now and then every `everyMs`, until the returned function is called. */
export function watchAmbient(everyMs: number): () => void {
    void refreshAmbient();
    const id = setInterval(() => void refreshAmbient(), everyMs);
    return () => clearInterval(id);
}
