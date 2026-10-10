// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Paired devices: every device paired with this computer's viewer (the host
// popover's "Pair a device"), when it paired and was last seen, and Revoke,
// which stops its token working and closes what it is watching. Below it,
// MuxBus sign-in (muxbus-sign-in.tsx) and Cloud presence (cloud-presence.tsx).
// agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §4.1.

import { Button } from "@/app/element/ui";
import { createEffect, createSignal, For, on, onMount, Show, type JSX } from "solid-js";

import { settingsAtom, viewerPairedAtom } from "@/app/store/global";
import { RpcApi, type ViewerDeviceInfo } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { CLOUD_PRESENCE_SETTING, CloudPresence } from "./cloud-presence";
import { MUXBUS_SIGN_IN_SETTING, MuxBusSignIn } from "./muxbus-sign-in";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const DEVICES_SETTINGS = {
    pairedDevices: {
        id: "devices.paired",
        label: "Paired devices",
        description:
            "Devices paired with this computer can watch its agents, read-only, over your local network. Revoke one to stop it.",
        section: "devices",
        keywords: ["mobile", "device", "tablet", "pair", "revoke", "viewer", "live feed", "qr code"],
    },
    muxbus: MUXBUS_SIGN_IN_SETTING,
    cloudPresence: CLOUD_PRESENCE_SETTING,
    publishPresence: {
        id: "devices.publish_presence",
        label: "Publish this computer to my devices",
        description:
            "While you are signed in, your devices can list this computer from anywhere. Turn it off to stop; your devices then show it as offline. Dev builds don't publish.",
        section: "devices",
        keywords: ["cloud", "presence", "publish", "offline", "mobile", "devices", "cloud:publishpresence"],
    },
    statusbarMuxbusCloud: {
        id: "devices.statusbar_muxbus_cloud",
        label: "Show MuxBus Cloud in the status bar",
        description:
            "The cloud dot next to the host name, and the sign-in block in the host menu. Turn it off if you don't use MuxBus Cloud; LAN is not affected. You can still sign in from Accounts, and a dead cloud sign-in is no longer flagged in the status bar.",
        section: "devices",
        keywords: ["muxbus", "cloud", "dot", "status bar", "statusbar", "hide", "sign in", "statusbar:showmuxbuscloud"],
    },
} satisfies Record<string, SettingsIndexEntry>;

/** "just now", "5 min ago", "3 h ago", or a date. */
export function ago(ms: number, nowMs: number): string {
    const s = Math.max(0, Math.floor((nowMs - ms) / 1000));
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.floor(s / 60)} min ago`;
    if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
    return new Date(ms).toLocaleDateString();
}

// ── Section: Paired devices ──────────────────────────────────────────────────

export function DevicesSection(): JSX.Element {
    const [devices, setDevices] = createSignal<ViewerDeviceInfo[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [revoking, setRevoking] = createSignal<string | null>(null);

    const load = async () => {
        try {
            const r = await RpcApi.ViewerDevicesCommand(TabRpcClient);
            setDevices(r.devices);
            setError(null);
        } catch (e) {
            setError(`Couldn't list paired devices: ${e instanceof Error ? e.message : e}`);
        }
    };
    onMount(() => void load());
    // A device paired while this is open shows up without a reopen.
    createEffect(on(viewerPairedAtom, () => void load(), { defer: true }));

    const revoke = async (device: ViewerDeviceInfo) => {
        setRevoking(device.device_id);
        try {
            await RpcApi.ViewerRevokeCommand(TabRpcClient, { device_id: device.device_id });
            setDevices((list) => (list ?? []).filter((d) => d.device_id !== device.device_id));
            setError(null);
        } catch (e) {
            setError(`Couldn't revoke ${device.device_name}: ${e instanceof Error ? e.message : e}`);
        } finally {
            setRevoking(null);
        }
    };

    const now = () => Date.now();

    return (
        <div class="settings-section-body">
            <SectionHeader label={DEVICES_SETTINGS.pairedDevices.label} />
            <div id={`setting-${DEVICES_SETTINGS.pairedDevices.id}`} class="setting-row">
                <div class="setting-devices-description">{DEVICES_SETTINGS.pairedDevices.description}</div>
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
                <Show when={devices()} fallback={<div class="setting-devices-empty">Loading…</div>}>
                    {(list) => (
                        <Show
                            when={list().length > 0}
                            fallback={
                                <div class="setting-devices-empty">
                                    No devices paired. Pair one from the host menu in the status bar (Pair a device).
                                </div>
                            }
                        >
                            <table class="setting-devices">
                                <thead>
                                    <tr>
                                        <th>Name</th>
                                        <th>Paired</th>
                                        <th>Last seen</th>
                                        <th />
                                    </tr>
                                </thead>
                                <tbody>
                                    <For each={list()}>
                                        {(device) => (
                                            <tr>
                                                <td class="setting-devices-name">{device.device_name}</td>
                                                <td>{ago(device.created_ms, now())}</td>
                                                <td>{device.last_seen_ms > 0 ? ago(device.last_seen_ms, now()) : "never"}</td>
                                                <td>
                                                    <Button
                                                        tone="danger"
                                                        disabled={revoking() === device.device_id}
                                                        onClick={() => void revoke(device)}
                                                    >
                                                        {revoking() === device.device_id ? "Revoking…" : "Revoke"}
                                                    </Button>
                                                </td>
                                            </tr>
                                        )}
                                    </For>
                                </tbody>
                            </table>
                        </Show>
                    )}
                </Show>
            </div>
            <SectionHeader label={DEVICES_SETTINGS.muxbus.label} />
            <MuxBusSignIn />
            <SectionHeader label={DEVICES_SETTINGS.cloudPresence.label} />
            <CloudPresence />
            <SettingRow
                id={DEVICES_SETTINGS.publishPresence.id}
                label={DEVICES_SETTINGS.publishPresence.label}
                description={DEVICES_SETTINGS.publishPresence.description}
                control={
                    <ToggleControl
                        checked={settingsAtom()?.["cloud:publishpresence"] !== false}
                        onChange={(v) => set("cloud:publishpresence", v)}
                    />
                }
            />
            <SettingRow
                id={DEVICES_SETTINGS.statusbarMuxbusCloud.id}
                label={DEVICES_SETTINGS.statusbarMuxbusCloud.label}
                description={DEVICES_SETTINGS.statusbarMuxbusCloud.description}
                control={
                    <ToggleControl
                        checked={settingsAtom()?.["statusbar:showmuxbuscloud"] !== false}
                        onChange={(v) => set("statusbar:showmuxbuscloud", v)}
                    />
                }
            />
        </div>
    );
}
