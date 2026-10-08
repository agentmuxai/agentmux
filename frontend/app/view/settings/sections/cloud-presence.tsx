// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Cloud presence: whether this computer's presence record reaches the cloud,
// which is how your signed-in devices list it when they aren't on this
// network, in one line, and "Publish now". The publisher and its states are
// crates/srv/src/muxbus/wan_presence.rs.

import { Button } from "@/app/element/ui";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { RpcApi, type PresenceStatusResult } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { SettingsIndexEntry } from "../settings-model";

export const CLOUD_PRESENCE_SETTING = {
    id: "devices.cloud_presence",
    label: "Cloud presence",
    description: "This computer tells the cloud it is online, so your signed-in devices can list it from anywhere.",
    section: "devices",
    keywords: ["cloud", "presence", "online", "publish", "relay", "remote", "offline", "mobile"],
} satisfies SettingsIndexEntry;

/** How often the line re-reads the status while it is shown. */
export const POLL_MS = 3000;

/** "12 s", "4 min", "2 h". */
export function span(ms: number): string {
    const s = Math.max(0, Math.round(ms / 1000));
    if (s < 60) return `${s} s`;
    if (s < 3600) return `${Math.round(s / 60)} min`;
    return `${Math.round(s / 3600)} h`;
}

/** "in 40 s", or "now" once it is due. */
function when(atMs: number | undefined, nowMs: number): string {
    return atMs == null || atMs - nowMs < 1000 ? "now" : `in ${span(atMs - nowMs)}`;
}

/** The status in one line of plain words. */
export function presenceLine(status: PresenceStatusResult, nowMs: number): string {
    const why = status.last_error ? ` (${status.last_error})` : "";
    switch (status.state) {
        case "signed_out":
            return "Sign in to publish";
        case "publishing":
            return status.last_ok_ms == null ? "Published" : `Published ${span(nowMs - status.last_ok_ms)} ago`;
        case "retrying":
            return `Retrying ${when(status.next_try_ms, nowMs)}${why}`;
        case "unsupported":
            return `The cloud doesn't accept presence yet. Checking again ${when(status.next_try_ms, nowMs)}`;
        case "rejected":
            return `The cloud refused this computer's record${why}. Trying again ${when(status.next_try_ms, nowMs)}`;
    }
}

export function CloudPresence(): JSX.Element {
    const [status, setStatus] = createSignal<PresenceStatusResult | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [now, setNow] = createSignal(Date.now());
    const [publishing, setPublishing] = createSignal(false);

    const load = async () => {
        try {
            setStatus(await RpcApi.PresenceStatusCommand(TabRpcClient));
            setError(null);
        } catch (e) {
            setStatus(null);
            setError(e instanceof Error ? e.message : String(e));
        }
        setNow(Date.now());
    };
    onMount(() => void load());
    const timer = setInterval(() => void load(), POLL_MS);
    onCleanup(() => clearInterval(timer));

    const publishNow = async () => {
        setPublishing(true);
        try {
            await RpcApi.PresencePublishNowCommand(TabRpcClient);
            await load();
        } catch (e) {
            setError(`Couldn't publish: ${e instanceof Error ? e.message : e}`);
        } finally {
            setPublishing(false);
        }
    };

    const line = () => {
        const s = status();
        return s ? presenceLine(s, now()) : (error() ?? "Loading…");
    };

    return (
        <div id={`setting-${CLOUD_PRESENCE_SETTING.id}`} class="setting-row">
            <div class="setting-devices-description">{CLOUD_PRESENCE_SETTING.description}</div>
            <div class="setting-presence">
                <span class="setting-presence-line" role="status">
                    {line()}
                </span>
                <Button
                    busy={publishing()}
                    disabled={status() == null || status()?.state === "signed_out"}
                    onClick={() => void publishNow()}
                >
                    Publish now
                </Button>
            </div>
            <Show when={status()?.note}>{(note) => <div class="setting-presence-note">{note()}</div>}</Show>
        </div>
    );
}
