// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useResumeRetryStream — the single per-block WPS subscription for
 * `agent-resume-retry`, published by the persistent controller's
 * stale-`--resume` recovery path (`retry_after_resume_failure` /
 * `publish_resume_retry_status` in `agentmux-srv`) so the pane can show a
 * "Reconnecting…" readout instead of going silent for the
 * seconds-to-tens-of-seconds it can take to detect a stale registry
 * `session_id` and respawn against a recovered one. See
 * docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2.
 *
 * Simpler than `useCompactionStream.ts`'s sibling hook: both the "retrying"
 * and "resolved" ends of this signal travel over this SAME WPS channel
 * (backend publishes with `persist: 2`, keeping the latest pair), so there's
 * no cross-channel staleness race to guard against here — a freshly
 * (re)subscribed pane simply reflects whichever status was published most
 * recently, which is always correct. No transcript node is pushed either;
 * this only drives the reducer's `reconnecting` pane-state axis (mirroring
 * `attachedTask`'s shape), not `AgentComposerStrip`'s "system" document.
 *
 * Installed at BODY scope by the caller, same early-return-safety rationale
 * as `useCompactionStream`/`useDockClearStream`.
 */

import { onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";

export interface UseResumeRetryStreamOptions {
    blockId: string;
    model: AgentPaneModel;
}

/**
 * Resolve a raw `agent-resume-retry` WPS payload into a dispatchable pane
 * command, or `null` to ignore it (malformed shape). Pure and exported for
 * direct unit coverage, same rationale as `resolveCompactionStart`.
 */
export function resolveResumeRetryEvent(
    data: unknown,
    now: number,
): { type: "ResumeRetryStarted"; at: number } | { type: "ResumeRetryResolved" } | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    if (d.status === "resolved") return { type: "ResumeRetryResolved" };
    if (d.status !== "retrying") return null;
    const at = typeof d.startedAt === "string" ? Date.parse(d.startedAt) : NaN;
    return { type: "ResumeRetryStarted", at: Number.isNaN(at) ? now : at };
}

export function useResumeRetryStream(opts: UseResumeRetryStreamOptions): void {
    const unsub = waveEventSubscribe({
        eventType: WpsEvent.AgentResumeRetry,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const command = resolveResumeRetryEvent(event?.data, Date.now());
            if (!command) return;
            // model.dispatchPane is the disposal-safe wrapper — required
            // here, not the raw (throwing) dispatch: this handler can fire
            // after the pane's slot unregisters, mirroring
            // useDockClearStream's identical reasoning.
            opts.model.dispatchPane(command, "system");
        },
    });

    // Own the subscription at body scope so it is torn down even if the
    // caller's onMount early-returns (e.g. enabled:false).
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
