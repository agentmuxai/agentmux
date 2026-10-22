// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { replaceProviderModels } from "./model-overlay";

/**
 * Codex's model list comes from the Codex CLI's cache of what the account
 * offers (`providers.models`), which a fresh sign-in only gets on its first
 * turn. So it is fetched at startup and again when a Codex pane's process
 * starts or its model menu opens, at most once per `MIN_INTERVAL_MS`: a local
 * file read on srv's side, no network.
 * SPEC_CODEX_OAUTH_IN_CONNECTORS_AND_EMAIL_LABELS_2026_10_22.md §4.
 */
export const MIN_INTERVAL_MS = 30_000;

let lastAt = Number.NEGATIVE_INFINITY;

export function refreshCodexModels(now: number = Date.now()): void {
    if (now - lastAt < MIN_INTERVAL_MS) return;
    lastAt = now;
    void Promise.resolve()
        .then(() => RpcApi.ProvidersModelsCommand(TabRpcClient, { provider_id: "codex" }))
        .then((res) =>
            replaceProviderModels(
                "codex",
                (res?.models ?? []).map((m) => ({ value: m.id, label: m.display_name })),
            ),
        )
        .catch(() => {
            // Best-effort: the curated list stays, and the next trigger retries.
            lastAt = Number.NEGATIVE_INFINITY;
        });
}

/** Forget the last fetch time (tests). */
export function _resetCodexModelsRefreshForTests(): void {
    lastAt = Number.NEGATIVE_INFINITY;
}
