// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Paired devices, the state: the devices as srv lists them, and the revoke and Can send calls that change one.

import { createSignal } from "solid-js";

import { createDomainStore } from "@/app/store/domain-store";
import { RpcApi, type ViewerDeviceInfo } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export type DevicesState = ReturnType<typeof createDevices>;

/** "just now", "5 min ago", "3 h ago", or a date. */
export function ago(ms: number, nowMs: number): string {
    const s = Math.max(0, Math.floor((nowMs - ms) / 1000));
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.floor(s / 60)} min ago`;
    if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
    return new Date(ms).toLocaleDateString();
}

// A device pairing (`viewer:paired`), a revoke or a change of its send grant (`viewerdevices:changed`),
// from this window, a paired device's own request or another window.
const store = createDomainStore<ViewerDeviceInfo[] | null>({
    initial: null,
    events: ["ViewerPaired", "ViewerDevicesChanged"],
    load: async () => (await RpcApi.ViewerDevicesCommand(TabRpcClient)).devices,
});

export function createDevices() {
    store.follow();
    const devices = store.data;
    const setDevices = (change: (list: ViewerDeviceInfo[] | null) => ViewerDeviceInfo[]) => store.set(change(store.data()));
    const [changeError, setError] = createSignal<string | null>(null);
    const error = () => changeError() ?? (store.error() && `Couldn't list paired devices: ${store.error()}`);
    const [revoking, setRevoking] = createSignal<string | null>(null);
    const [granting, setGranting] = createSignal<string | null>(null);

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

    const setCanSend = async (device: ViewerDeviceInfo, canSend: boolean) => {
        setGranting(device.device_id);
        try {
            const r = await RpcApi.ViewerCanSendCommand(TabRpcClient, { device_id: device.device_id, can_send: canSend });
            setDevices((list) =>
                (list ?? []).map((d) => (d.device_id === device.device_id ? { ...d, can_send: r.can_send } : d))
            );
            setError(null);
        } catch (e) {
            setError(`Couldn't change ${device.device_name}: ${e instanceof Error ? e.message : e}`);
        } finally {
            setGranting(null);
        }
    };

    const now = () => Date.now();

    return { devices, error, revoking, granting, revoke, setCanSend, now };
}
