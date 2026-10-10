// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Cloud presence: whether this computer's presence record reaches the cloud,
// which is how your signed-in devices list it when they aren't on this
// network, in one line, and "Publish now". The publisher and its states are
// crates/srv/src/muxbus/wan_presence.rs. The "Publish this computer to my
// devices" switch below it is devices-section.tsx's.

import { Button } from "@/app/element/ui";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { RpcApi, type PresenceOffReason, type PresenceStatusResult } from "@/app/store/rpc-api";
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

/** Why an install doesn't publish, after "Off". */
const OFF_REASONS: Record<PresenceOffReason, string> = {
    setting: "Off (setting)",
    dev_build: "Off for dev builds",
    headless: "Off for headless installs",
    isolated_home: "Off for isolated homes",
    test_harness: "Off under test",
};

/** States in which "Publish now" has nothing to do. */
const NOTHING_TO_PUBLISH: ReadonlySet<PresenceStatusResult["state"]> = new Set(["signed_out", "signed_off", "off"]);

/** The status in one line of plain words. */
export function presenceLine(status: PresenceStatusResult, nowMs: number): string {
    const why = status.last_error ? ` (${status.last_error})` : "";
    switch (status.state) {
        case "signed_out":
            return "Sign in to publish";
        case "signed_off":
            return "Signed off: your devices show this computer as offline";
        case "off":
            return status.off_reason ? OFF_REASONS[status.off_reason] : "Off";
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

    const canPublish = () => {
        const s = status();
        return s != null && !NOTHING_TO_PUBLISH.has(s.state);
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
                    disabled={!canPublish()}
                    onClick={() => void publishNow()}
                >
                    Publish now
                </Button>
            </div>
            <Show when={status()?.note}>{(note) => <div class="setting-presence-note">{note()}</div>}</Show>
        </div>
    );
}
