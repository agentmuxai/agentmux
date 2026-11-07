// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Paired devices, the views: the table of devices over a state from devices-section-state.ts.

import { Button, Switch } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";

import { tip } from "@/app/element/tip";
import type { ViewerDeviceInfo } from "@/app/store/rpc-api";
import { ago, type DevicesState } from "./devices-section-state";

function DeviceRow(props: { s: DevicesState; device: ViewerDeviceInfo }): JSX.Element {
    const { s, device } = props;
    return (
        <tr>
            <td class="setting-devices-name">{device.device_name}</td>
            <td>{ago(device.created_ms, s.now())}</td>
            <td>{device.last_seen_ms > 0 ? ago(device.last_seen_ms, s.now()) : "never"}</td>
            <td>
                <Show
                    when={device.has_key}
                    fallback={
                        <span
                            class="setting-devices-muted"
                            {...tip("This device paired before devices had keys. Pair it again to let it send.")}
                        >
                            Pair again
                        </span>
                    }
                >
                    <Switch
                        checked={device.can_send}
                        disabled={s.granting() === device.device_id}
                        ariaLabel={`${device.device_name} can send messages`}
                        onChange={(v) => void s.setCanSend(device, v)}
                    />
                </Show>
            </td>
            <td>
                <Button
                    tone="danger"
                    disabled={s.revoking() === device.device_id}
                    onClick={() => void s.revoke(device)}
                >
                    {s.revoking() === device.device_id ? "Revoking…" : "Revoke"}
                </Button>
            </td>
        </tr>
    );
}

function DevicesTable(props: { s: DevicesState; list: () => ViewerDeviceInfo[] }): JSX.Element {
    return (
        <Show
            when={props.list().length > 0}
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
                        <th>Can send</th>
                        <th />
                    </tr>
                </thead>
                <tbody>
                    <For each={props.list()}>{(device) => <DeviceRow s={props.s} device={device} />}</For>
                </tbody>
            </table>
        </Show>
    );
}

/** The Paired devices row: its description, what went wrong, and the table of devices (or that they are loading). */
export function PairedDevices(props: { s: DevicesState; description: string; id: string }): JSX.Element {
    const s = props.s;
    return (
        <div id={`setting-${props.id}`} class="setting-row">
            <div class="setting-devices-description">{props.description}</div>
            <Show when={s.error()}>
                <div class="settings-config-error" role="alert">
                    {s.error()}
                </div>
            </Show>
            <Show when={s.devices()} fallback={<div class="setting-devices-empty">Loading…</div>}>
                {(list) => <DevicesTable s={s} list={list} />}
            </Show>
        </div>
    );
}
