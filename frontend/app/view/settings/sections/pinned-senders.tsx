// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Pinned senders: the first key seen for an agent's name on your network, and
// the first install seen speaking for it through your cloud account, are
// pinned, and a message from anything else is refused. This lists the pins,
// shows what was refused beside each, and lets you accept the new one or
// forget the pin so the next one seen is pinned. The pins decide who may speak
// as an agent, so only this window can change them
// (crates/srv/src/server/app_api/pins.rs).

import { Button } from "@/app/element/ui";
import { createSignal, For, Show, type JSX } from "solid-js";

import { createDomainStore } from "@/app/store/domain-store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import type { PinnedSendersResult } from "@/types/rpc/PinnedSendersResult";
import type { SettingsIndexEntry } from "../settings-model";

export const PINNED_SENDERS_SETTING = {
    id: "devices.pinned_senders",
    label: "Pinned senders",
    description:
        "The first key AgentMux sees for an agent's name on your network, and the first of your installs it sees speaking for that name through your cloud account, are remembered, and a message from anything else is refused as unverified. If an agent really did change (it was recreated, or moved to another computer), accept the new one here. Forget a pin to trust whatever speaks for that name next.",
    section: "devices",
    keywords: ["pin", "pinned", "key", "sender", "agent", "network", "lan", "cloud", "install", "spoof", "verify", "trust", "reset"],
} satisfies SettingsIndexEntry;

/** What one button press does to one pin. */
export type PinChange = { kind: "lan" | "wan"; agentId: string; channel?: string; how: "adopt" | "forget"; expect?: string };

/** "3 days ago", "just now": when a pin was set or a refusal last seen. */
export function ago(epochSecs: number, nowSecs: number): string {
    const s = Math.max(0, nowSecs - epochSecs);
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.round(s / 60)} min ago`;
    if (s < 86400) return `${Math.round(s / 3600)} h ago`;
    return `${Math.round(s / 86400)} days ago`;
}

const same = (a: PinChange | null, b: PinChange) =>
    a != null && a.kind === b.kind && a.agentId === b.agentId && a.channel === b.channel && a.how === b.how && a.expect === b.expect;

// A pin changes here (`pins:changed`), and a refusal is counted without any event, so the list is
// also reloaded every half minute while it is open.
const store = createDomainStore<PinnedSendersResult | null>({
    initial: null,
    events: ["PinsChanged"],
    safetyPollMs: 30_000,
    load: () => RpcApi.PinsListCommand(TabRpcClient),
});

/** What a LAN pin shows: its key and when it was pinned, or, for a name with no pin yet, that no install you trust vouched
 *  for the key shown, and the key refused (a different one) or shown (the first). */
function LanPinDetails(props: { pin: LanPinInfo; now: number }): JSX.Element {
    const pinned = () => props.pin.key_fp !== "";
    return (
        <>
            <Show when={pinned()} fallback={<div>Not pinned yet: no install you trust vouched for the key below.</div>}>
                <div>
                    Key {props.pin.key_fp}, pinned {ago(props.pin.pinned_at, props.now)}
                </div>
            </Show>
            <Show when={props.pin.refused}>
                {(r) => (
                    <div class="setting-pin-refused" role="status">
                        {pinned() ? "A different key" : "A key"}, {r().key_fp}, was {pinned() ? "refused" : "shown"} {r().count} time
                        {r().count === 1 ? "" : "s"}; last {ago(r().last_seen_at, props.now)}.
                        {pinned() ? "" : " Its messages arrive unproven until you accept it."}
                    </div>
                )}
            </Show>
        </>
    );
}

export function PinnedSenders(): JSX.Element {
    store.follow();
    const pins = store.data;
    const [changeError, setChangeError] = createSignal<string | null>(null);
    const error = () => changeError() ?? store.error();
    const [asking, setAsking] = createSignal<PinChange | null>(null);
    const [busy, setBusy] = createSignal(false);
    const now = () => Math.floor(Date.now() / 1000);

    // The first press asks, the second does it: a pin decides who may speak as an agent.
    const press = async (change: PinChange) => {
        if (!same(asking(), change)) {
            setAsking(change);
            return;
        }
        setAsking(null);
        setBusy(true);
        // Adopt names what was on screen: srv refuses it if something else was refused since.
        const data = { kind: change.kind, agent_id: change.agentId, channel: change.channel, expect: change.expect };
        try {
            store.set(
                change.how === "adopt"
                    ? await RpcApi.PinsAdoptCommand(TabRpcClient, data)
                    : await RpcApi.PinsForgetCommand(TabRpcClient, data)
            );
            setChangeError(null);
        } catch (e) {
            setChangeError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    // `shown`: the refused key or install on screen beside the pin, if any.
    // `first`: the name has no pin yet, so there is nothing to forget and the key is the first one to accept.
    const actions = (change: Omit<PinChange, "how" | "expect">, shown?: string, first = false): JSX.Element => (
        <span class="setting-pins-actions">
            <Show when={shown}>
                <Button disabled={busy()} onClick={() => void press({ ...change, how: "adopt", expect: shown })}>
                    {same(asking(), { ...change, how: "adopt", expect: shown }) ? "Press again to accept" : first ? "Accept this key" : "Accept the new one"}
                </Button>
            </Show>
            <Show when={!first}>
                <Button disabled={busy()} onClick={() => void press({ ...change, how: "forget" })}>
                    {same(asking(), { ...change, how: "forget" }) ? "Press again to forget" : "Forget"}
                </Button>
            </Show>
        </span>
    );

    return (
        <div id={`setting-${PINNED_SENDERS_SETTING.id}`} class="setting-row">
            <div class="setting-devices-description">{PINNED_SENDERS_SETTING.description}</div>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
            <Show when={pins()} fallback={<Show when={!error()}><div class="setting-devices-empty">Loading…</div></Show>}>
                {(all) => (
                    <Show
                        when={all().lan.length + all().wan.length > 0}
                        fallback={<div class="setting-devices-empty">Nothing is pinned yet. A name is pinned the first time another computer speaks for it.</div>}
                    >
                        <ul class="setting-pins">
                            <For each={all().lan}>
                                {(pin) => (
                                    <li class="setting-pin" data-kind="lan">
                                        <div class="setting-pin-name">{pin.agent_id} <span class="setting-devices-muted">on your network</span></div>
                                        <LanPinDetails pin={pin} now={now()} />
                                        {actions({ kind: "lan", agentId: pin.agent_id }, pin.refused?.key_fp, !pin.key_fp)}
                                    </li>
                                )}
                            </For>
                            <For each={all().wan}>
                                {(pin) => (
                                    <li class="setting-pin" data-kind="wan">
                                        <div class="setting-pin-name">
                                            {pin.agent_id} <span class="setting-devices-muted">through your account, {pin.channel}</span>
                                        </div>
                                        <div>
                                            Pinned to {pin.instance} {ago(pin.pinned_at, now())}
                                        </div>
                                        <Show when={pin.refused}>
                                            {(r) => (
                                                <div class="setting-pin-refused" role="status">
                                                    {r().instance} spoke for it {r().count} time{r().count === 1 ? "" : "s"} and was refused; last{" "}
                                                    {ago(r().last_seen_at, now())}.
                                                </div>
                                            )}
                                        </Show>
                                        {actions({ kind: "wan", agentId: pin.agent_id, channel: pin.channel }, pin.refused?.instance)}
                                    </li>
                                )}
                            </For>
                        </ul>
                    </Show>
                )}
            </Show>
        </div>
    );
}
