// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useSessionDigest — owns the AI-generated session summary banner state,
 * the RPC that generates it, and the auto-trigger that decides whether
 * to show or generate one on pane open.
 *
 * Step 6 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Behavior on mount:
 *   - Read block meta for cached digest + idle time + line counts
 *   - If a cached digest exists, surface it immediately
 *   - If idle >1h AND >=20 new lines since last digest, fire fetch()
 *     in the background (non-blocking, non-forced)
 *
 * Returns:
 *   - summary       — current digest text, or null if none/dismissed
 *   - generatedAt   — Unix ms timestamp when the cached digest was made
 *   - loading       — true while a fetch is in flight
 *   - dismissed     — user clicked "X" on the banner
 *   - fetch(force)  — async generate. force=true bypasses cache
 *   - dismiss()     — hide the banner for this session
 */

import { createSignal, onMount, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import type { LogFn } from "../types";
export type { LogFn };

const IDLE_THRESHOLD_MS = 3_600_000;     // 1 hour
const STALE_LINE_THRESHOLD = 20;          // lines of new activity since last digest

export interface UseSessionDigestOptions {
    blockId: string;
    block: Accessor<{ meta?: Record<string, unknown> } | undefined>;
    log: LogFn;
}

export interface UseSessionDigest {
    summary: Accessor<string | null>;
    generatedAt: Accessor<number | null>;
    loading: Accessor<boolean>;
    dismissed: Accessor<boolean>;
    fetch: (force?: boolean) => Promise<void>;
    dismiss: () => void;
}

export function useSessionDigest(opts: UseSessionDigestOptions): UseSessionDigest {
    const [summary, setSummary] = createSignal<string | null>(null);
    const [generatedAt, setGeneratedAt] = createSignal<number | null>(null);
    const [loading, setLoading] = createSignal(false);
    const [dismissed, setDismissed] = createSignal(false);

    const fetch = async (force = false): Promise<void> => {
        if (loading()) return;
        setLoading(true);
        try {
            const result = await RpcApi.SessionDigestCommand(TabRpcClient, {
                block_id: opts.blockId,
                force,
            }, { timeout: 90_000 }); // 60s CLI + headroom

            if (result.summary) {
                setSummary(result.summary);
                setGeneratedAt(result.generated_at > 0 ? result.generated_at : null);
            } else {
                setSummary(null);
            }
        } catch (err: any) {
            opts.log("digest", `failed to generate session digest: ${err?.message ?? String(err)}`, "warn");
            setSummary(null);
        } finally {
            setLoading(false);
        }
    };

    const dismiss = () => setDismissed(true);

    onMount(() => {
        const meta = opts.block()?.meta ?? {};
        const lastActivityMs = typeof meta["session:last_activity_ms"] === "number"
            ? (meta["session:last_activity_ms"] as number)
            : 0;
        const lineCount = typeof meta["session:line_count"] === "number"
            ? (meta["session:line_count"] as number)
            : 0;
        const cachedDigest = typeof meta["session:digest_summary"] === "string"
            ? (meta["session:digest_summary"] as string)
            : null;
        const cachedDigestAt = typeof meta["session:digest_generated_at"] === "number"
            ? (meta["session:digest_generated_at"] as number)
            : 0;
        const digestLastLineCount = typeof meta["session:digest_last_line_count"] === "number"
            ? (meta["session:digest_last_line_count"] as number)
            : 0;

        const idleMs = lastActivityMs > 0 ? Date.now() - lastActivityMs : 0;
        const idleOverThreshold = idleMs > IDLE_THRESHOLD_MS;
        const linesSinceDigest = lineCount - digestLastLineCount;

        if (cachedDigest) {
            setSummary(cachedDigest);
            setGeneratedAt(cachedDigestAt > 0 ? cachedDigestAt : null);
            if (idleOverThreshold && linesSinceDigest >= STALE_LINE_THRESHOLD) {
                fetch(false);
            }
        } else if (idleOverThreshold && lineCount > STALE_LINE_THRESHOLD) {
            fetch(false);
        }
    });

    return { summary, generatedAt, loading, dismissed, fetch, dismiss };
}
