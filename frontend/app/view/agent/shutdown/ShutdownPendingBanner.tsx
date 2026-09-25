// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's "shutting down in 15 s — Keep running" banner
 * (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5): shown when something
 * other than the user asked to shut this agent down. Counts down live, chimes
 * the falling shutdown tone at the start and with 5 s left, and "Keep
 * running" cancels it through `agentshutdownkeep`.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { muxEventSubscribe } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { playShutdownPendingTone } from "@/app/notification/sound/sound-service";
import { PaneRow } from "../components/PaneRow";
import {
    clearedRequestId,
    EVENT_SHUTDOWN_PENDING,
    EVENT_SHUTDOWN_PENDING_CLEARED,
    LAST_CALL_MS,
    parsePending,
    pendingTitle,
    replayPending,
    secondsLeft,
    type PendingShutdown,
} from "./shutdown-pending";

interface ShutdownPendingBannerProps {
    blockId: string;
    agentId: string;
    agentName: string;
}

export const ShutdownPendingBanner = (props: ShutdownPendingBannerProps): JSX.Element => {
    const [pending, setPending] = createSignal<PendingShutdown | null>(null);
    const [now, setNow] = createSignal(Date.now());
    const [keeping, setKeeping] = createSignal(false);
    let lastCallFor: string | null = null;
    // Set by any live event: from then on it, not the history replay, is the
    // truth (a replay resolving after a live -cleared must not bring the
    // banner back).
    let sawLive = false;

    const show = (p: PendingShutdown | null, chime: boolean) => {
        setPending(p);
        setKeeping(false);
        setNow(Date.now());
        if (p && chime) playShutdownPendingTone();
    };

    const scope = `block:${props.blockId}`;
    const unsub = muxEventSubscribe(
        {
            eventType: EVENT_SHUTDOWN_PENDING,
            scope,
            handler: (event: MuxEvent) => {
                sawLive = true;
                show(parsePending(event?.data), true);
            },
        },
        {
            eventType: EVENT_SHUTDOWN_PENDING_CLEARED,
            scope,
            handler: (event: MuxEvent) => {
                sawLive = true;
                if (clearedRequestId(event?.data) === pending()?.request_id) setPending(null);
            },
        },
    );
    onCleanup(unsub);

    // A pane opened mid-countdown still shows it: both events are persisted.
    const history = (event: string) =>
        RpcApi.EventReadHistoryCommand(TabRpcClient, { event, scope, maxitems: 1 })
            .then((events) => events?.[events.length - 1]?.data ?? null)
            .catch(() => null);
    void Promise.all([history(EVENT_SHUTDOWN_PENDING), history(EVENT_SHUTDOWN_PENDING_CLEARED)]).then(
        ([p, cleared]) => {
            if (sawLive) return;
            const replayed = replayPending(p, cleared, Date.now());
            if (replayed) show(replayed, false);
        },
    );

    const timer = setInterval(() => {
        const p = pending();
        if (!p) return;
        const t = Date.now();
        setNow(t);
        if (p.deadline_ms - t <= LAST_CALL_MS && p.deadline_ms > t && lastCallFor !== p.request_id) {
            lastCallFor = p.request_id;
            playShutdownPendingTone();
        }
    }, 250);
    onCleanup(() => clearInterval(timer));

    const keep = () => {
        const p = pending();
        if (!p || keeping()) return;
        setKeeping(true);
        RpcApi.AgentShutdownKeepCommand(TabRpcClient, { blockid: props.blockId, request_id: p.request_id })
            .then((res) => {
                // `too_late`: the shutdown has begun; the close log takes over.
                if (res?.outcome === "kept_by_user" || res?.outcome === "too_late") setPending(null);
            })
            .catch((e) => {
                console.error("[shutdown] keep running failed:", e);
                setKeeping(false);
            });
    };

    return (
        <Show when={pending()}>
            {(p) => (
                <PaneRow
                    sigil="⏻"
                    accent="error"
                    title={`${pendingTitle(p(), props.agentId, props.agentName)}. Closing in ${secondsLeft(p(), now())} s.`}
                    actions={[
                        {
                            label: keeping() ? "Keeping…" : "Keep running",
                            title: "Cancel this shutdown and keep the agent running",
                            primary: true,
                            disabled: keeping(),
                            onClick: keep,
                        },
                    ]}
                />
            )}
        </Show>
    );
};

ShutdownPendingBanner.displayName = "ShutdownPendingBanner";
