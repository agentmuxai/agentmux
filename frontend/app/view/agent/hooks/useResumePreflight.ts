// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useResumePreflight — ask, at pane mount, whether this agent's next turn will
 * continue the conversation on screen or start a new one.
 *
 * Every other continuity signal in the pane is retrospective: the
 * `agentmux_session_outcome` divider, the `session:resume_failed` banner and
 * the "Reconnecting…" readout all describe a resume that has already been
 * attempted, which can only happen once the user has sent something. That
 * produces the experience this hook exists to remove — the pane opens showing
 * a long prior conversation, the user types, and only then does the transcript
 * clear and announce a new session.
 *
 * The backend can answer this without spawning anything (a `--resume` fails
 * exactly when the session file isn't under the CLI's config dir), so this is
 * one read-only RPC on mount. See `agentmux-srv/src/backend/resume_preflight.rs`.
 *
 * Deliberately fire-and-forget: no retry, no polling, no refetch on
 * `agent:sessionid` changes. Once the pane has spawned, the retrospective
 * signals above are authoritative and strictly better informed than this
 * prediction — a preflight that kept re-running would eventually contradict
 * them. The verdict describes the pane as the user found it, and that's all.
 */

import { createSignal, onCleanup } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/**
 * How long to let the preflight run before showing its progress list. The
 * common case is one or two `stat`s and resolves far inside this, so the list
 * never appears; a cold or network-backed home directory is what it's for.
 */
const STEPS_VISIBLE_AFTER_MS = 200;

export interface UseResumePreflightResult {
    /** `null` until the RPC resolves, and on any failure. */
    result: () => SessionResumePreflightResult | null;
    /** True once the preflight has been running longer than the reveal delay. */
    showSteps: () => boolean;
    /** True from mount until the RPC settles either way. */
    pending: () => boolean;
}

export function useResumePreflight(blockId: string): UseResumePreflightResult {
    const [result, setResult] = createSignal<SessionResumePreflightResult | null>(null);
    const [showSteps, setShowSteps] = createSignal(false);
    const [pending, setPending] = createSignal(true);

    const revealTimer = setTimeout(() => {
        if (pending()) setShowSteps(true);
    }, STEPS_VISIBLE_AFTER_MS);

    let disposed = false;
    onCleanup(() => {
        disposed = true;
        clearTimeout(revealTimer);
    });

    RpcApi.SessionResumePreflightCommand(TabRpcClient, { block_id: blockId })
        .then((r) => {
            if (disposed) return;
            setResult(r);
        })
        .catch((e) => {
            // A failed preflight must be indistinguishable from "we don't
            // know" — never a warning of its own. Predicting continuity is a
            // convenience; being wrong about it is worse than being silent.
            console.warn("resume preflight failed:", e);
        })
        .finally(() => {
            if (disposed) return;
            setPending(false);
            setShowSteps(false);
            clearTimeout(revealTimer);
        });

    return { result, showSteps, pending };
}
