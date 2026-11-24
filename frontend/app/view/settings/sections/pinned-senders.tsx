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
import { t } from "@/app/i18n/i18n";
import { createSignal, For, Show, type JSX } from "solid-js";

import { createDomainStore } from "@/app/store/domain-store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import type { WanPinInfo } from "@/types/rpc/WanPinInfo";
import type { PinnedSendersResult } from "@/types/rpc/PinnedSendersResult";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";

export const PINNED_SENDERS_SETTING = settingEntry({
    id: "devices.pinned_senders",
    labelKey: "settings.devices.pinnedSenders.label",
    descriptionKey: "settings.devices.pinnedSenders.description",
    section: "devices",
    keywords: ["pin", "pinned", "key", "sender", "agent", "network", "lan", "cloud", "install", "spoof", "verify", "trust", "reset"],
});

/** What one button press does to one pin. */
export type PinChange = { kind: "lan" | "wan"; agentId: string; channel?: string; how: "adopt" | "forget"; expect?: string };

/** "3 days ago", "just now": when a pin was set or a refusal last seen. */
export function ago(epochSecs: number, nowSecs: number): string {
    const s = Math.max(0, nowSecs - epochSecs);
    if (s < 60) return t("settings.pins.ago.now");
    if (s < 3600) return t("settings.pins.ago.minutes", { n: Math.round(s / 60) });
    if (s < 86400) return t("settings.pins.ago.hours", { n: Math.round(s / 3600) });
    return t("settings.pins.ago.days", { count: Math.round(s / 86400) });
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
            <Show when={pinned()} fallback={<div>{t("settings.pins.lan.notPinned")}</div>}>
                <div>
                    {t("settings.pins.lan.pinned", { fp: props.pin.key_fp, ago: ago(props.pin.pinned_at, props.now) })}
                </div>
                <div>
                    {props.pin.bound_to
                        ? t("settings.pins.lan.boundTo", { install: props.pin.bound_to })
                        : t("settings.pins.lan.unbound")}
                </div>
            </Show>
            <Show when={props.pin.refused}>
                {(r) => (
                    <div class="setting-pin-refused" role="status">
                        {t(pinned() ? "settings.pins.lan.refusedDifferent" : "settings.pins.lan.refusedShown", {
                            fp: r().key_fp,
                            count: r().count,
                            ago: ago(r().last_seen_at, props.now),
                        })}{" "}
                        {pinned() ? t("settings.pins.lan.acceptNote") : t("settings.pins.lan.unprovenNote")}
                        <Show when={r().why}>{(why) => <> {t("settings.pins.lan.why", { why: why() })}</>}</Show>
                    </div>
                )}
            </Show>
        </>
    );
}

/** What a cloud pin shows: the install it is pinned to and when, and what was refused beside it. */
function WanPinDetails(props: { pin: WanPinInfo; now: number }): JSX.Element {
    return (
        <>
            <div class="setting-pin-name">
                {props.pin.agent_id}{" "}
                <span class="setting-devices-muted">{t("settings.pins.wan.through", { channel: props.pin.channel })}</span>
            </div>
            <div>{t("settings.pins.wan.pinned", { instance: props.pin.instance, ago: ago(props.pin.pinned_at, props.now) })}</div>
            <Show when={props.pin.refused}>
                {(r) => (
                    <div class="setting-pin-refused" role="status">
                        {t("settings.pins.wan.refused", {
                            instance: r().instance,
                            count: r().count,
                            ago: ago(r().last_seen_at, props.now),
                        })}
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
                    {same(asking(), { ...change, how: "adopt", expect: shown })
                        ? t("settings.pins.pressAccept")
                        : first
                          ? t("settings.pins.acceptKey")
                          : t("settings.pins.acceptNew")}
                </Button>
            </Show>
            <Show when={!first}>
                <Button disabled={busy()} onClick={() => void press({ ...change, how: "forget" })}>
                    {same(asking(), { ...change, how: "forget" }) ? t("settings.pins.pressForget") : t("settings.pins.forget")}
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
            <Show when={pins()} fallback={<Show when={!error()}><div class="setting-devices-empty">{t("settings.pins.loading")}</div></Show>}>
                {(all) => (
                    <Show
                        when={all().lan.length + all().wan.length > 0}
                        fallback={<div class="setting-devices-empty">{t("settings.pins.none")}</div>}
                    >
                        <ul class="setting-pins">
                            <For each={all().lan}>
                                {(pin) => (
                                    <li class="setting-pin" data-kind="lan">
                                        <div class="setting-pin-name">{pin.agent_id} <span class="setting-devices-muted">{t("settings.pins.onNetwork")}</span></div>
                                        <LanPinDetails pin={pin} now={now()} />
                                        {actions({ kind: "lan", agentId: pin.agent_id }, pin.refused?.key_fp, !pin.key_fp)}
                                    </li>
                                )}
                            </For>
                            <For each={all().wan}>
                                {(pin) => (
                                    <li class="setting-pin" data-kind="wan">
                                        <WanPinDetails pin={pin} now={now()} />
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
