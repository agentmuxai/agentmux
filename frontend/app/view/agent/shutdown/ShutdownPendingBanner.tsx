// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's "shutting down in N s — Keep running / Close now" banner
 * (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5): shown when something
 * other than the user asked to shut this agent down. Counts down live, chimes
 * the falling shutdown tone at the start and with 5 s left, "Keep
 * running" cancels it through `agentshutdownkeep`, and "Close now" ends the wait
 * through `agentshutdownnow` (the window's alone: srv proves it)
 * (docs/specs/SPEC_COUNTDOWN_BUTTONS_AND_BATCH_AGENT_ACTIONS_2026_10_28.md §3).
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { playShutdownPendingTone } from "@/app/notification/sound/sound-service";
import { PaneRow } from "../components/PaneRow";
import {
    EVENT_SHUTDOWN_PENDING,
    EVENT_SHUTDOWN_PENDING_CLEARED,
    LAST_CALL_MS,
    countdownLine,
    parsePending,
    pendingTitle,
    replayPending,
    secondsLeft,
    type PendingShutdown,
} from "./shutdown-pending";
import { readMuxEventHistory } from "@/app/store/mps-history";

interface ShutdownPendingBannerProps {
    blockId: string;
    agentId: string;
    agentName: string;
}

export const ShutdownPendingBanner = (props: ShutdownPendingBannerProps): JSX.Element => {
    const [pending, setPending] = createSignal<PendingShutdown | null>(null);
    const [now, setNow] = createSignal(Date.now());
    const [keeping, setKeeping] = createSignal(false);
    const [closing, setClosing] = createSignal(false);
    let lastCallFor: string | null = null;
    // Set by any live event: from then on it, not the history replay, is the
    // truth (a replay resolving after a live -cleared must not bring the
    // banner back).
    let sawLive = false;

    const show = (p: PendingShutdown | null, chime: boolean) => {
        setPending(p);
        setKeeping(false);
        setClosing(false);
        setNow(Date.now());
        if (p && chime) playShutdownPendingTone();
    };

    const scope = `block:${props.blockId}`;
    const unsubPending = onMuxEvent(EVENT_SHUTDOWN_PENDING, (view) => {
        sawLive = true;
        const next = parsePending(view, props.blockId);
        // The same request again (another agent of its batch was kept) updates the line, without a chime.
        show(next, next?.request_id !== pending()?.request_id);
    }, scope);
    const unsubCleared = onMuxEvent(EVENT_SHUTDOWN_PENDING_CLEARED, (cleared) => {
        sawLive = true;
        if (cleared.request_id === pending()?.request_id) setPending(null);
    }, scope);
    onCleanup(() => {
        unsubPending();
        unsubCleared();
    });

    // A pane opened mid-countdown still shows it: both events are persisted.
    const last = <T,>(items: T[]): T | null => items[items.length - 1] ?? null;
    void Promise.all([
        readMuxEventHistory(EVENT_SHUTDOWN_PENDING, scope, 1).then(last).catch(() => null),
        readMuxEventHistory(EVENT_SHUTDOWN_PENDING_CLEARED, scope, 1).then(last).catch(() => null),
    ]).then(
        ([p, cleared]) => {
            if (sawLive) return;
            const replayed = replayPending(p, cleared, Date.now(), props.blockId);
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
        if (!p || keeping() || closing()) return;
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

    const closeNow = () => {
        const p = pending();
        if (!p || keeping() || closing()) return;
        setClosing(true);
        RpcApi.AgentShutdownNowCommand(TabRpcClient, { blockid: props.blockId, request_id: p.request_id })
            .then((res) => {
                // `too_late`: it was kept or has begun; the cleared event settles the banner.
                if (res?.outcome === "too_late") setClosing(false);
            })
            .catch((e) => {
                console.error("[shutdown] close now failed:", e);
                setClosing(false);
            });
    };

    return (
        <Show when={pending()}>
            {(p) => (
                <PaneRow
                    operatorOnly
                    sigil="⏻"
                    accent="error"
                    title={`${pendingTitle(p(), props.agentId, props.agentName)}. ${countdownLine(p(), secondsLeft(p(), now()))}`}
                    actions={[
                        {
                            label: keeping() ? "Keeping…" : "Keep running",
                            title: p().others > 0 ? "Keep this agent running; the others still close" : "Cancel this shutdown and keep the agent running",
                            primary: true,
                            disabled: keeping() || closing(),
                            onClick: keep,
                        },
                        {
                            label: closing() ? "Closing…" : p().effect === "stop" ? "Stop now" : "Close now",
                            title: p().others > 0 ? "Don't wait for the countdown: close all of them now" : "Don't wait for the countdown: do it now",
                            disabled: keeping() || closing(),
                            onClick: closeNow,
                        },
                    ]}
                />
            )}
        </Show>
    );
};

ShutdownPendingBanner.displayName = "ShutdownPendingBanner";
