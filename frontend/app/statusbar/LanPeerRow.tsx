// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One discovered LAN peer in the network pill's popover: its name and version on the line, and,
// opened, where it is, what it runs, who is on it and whether a message from its agents was refused
// for a key that changed. Re-pinning stays in Settings > Devices > Pinned senders, beside its
// explanation (docs/specs/SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §5).

import { For, Index, Show, createMemo, createSignal, onMount, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { Button } from "@/app/element/ui";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { LanInstance } from "@/types/rpc/LanInstance";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import {
    hostsWithSeveralInstances,
    lastSeenText,
    middleTruncate,
    osAndChannel,
    peerKeyRefusals,
    showsChannel,
} from "./lan-peer";

const openPinnedSenders = () =>
    void openOrFocusPaneByView(
        "settings",
        { meta: { view: "settings", "settings:section": "devices" } },
        { "settings:section": "devices" }
    );

const openSwarm = () => void openOrFocusPaneByView("swarm");

/** Longest channel shown on the line; a longer one (a local build's) is cut in the middle. */
const CHANNEL_MAX_CHARS = 18;

/** Which peer a row is, across the fresh objects each `laninstances` event brings. */
const peerKey = (peer: LanInstance): string => peer.instance_id || `${peer.address}:${peer.port}`;

export function LanPeerRow(props: {
    peer: LanInstance;
    pins: readonly LanPinInfo[];
    /** Whether the line names the peer's channel (its host runs more than one instance). */
    showChannel?: boolean;
    open: boolean;
    onToggle: () => void;
}): JSX.Element {
    const tick = useTick(1000);
    const nowSecs = createMemo(() => (tick(), Math.floor(Date.now() / 1000)));
    const refusals = createMemo(() => peerKeyRefusals(props.peer, props.pins));
    const name = () => props.peer.hostname || props.peer.instance_id || "resolving…";
    const platform = () => osAndChannel(props.peer);
    const agentCount = () => props.peer.agents.length;
    return (
        <div class="lan-peer" data-testid="lan-peer">
            <Button
                tone="quiet"
                density="compact"
                class="lan-peer-head"
                icon={props.open ? "chevron-down" : "chevron-right"}
                aria-expanded={props.open}
                onClick={props.onToggle}
            >
                <span class="lan-peer-name">{name()}</span>
                <Show when={props.peer.version}>
                    <span class="status-bar-popover-mono lan-peer-version">v{props.peer.version}</span>
                </Show>
                <Show when={props.showChannel && props.peer.channel}>
                    <span class="status-bar-popover-mono lan-peer-channel" title={props.peer.channel}>
                        {middleTruncate(props.peer.channel, CHANNEL_MAX_CHARS)}
                    </span>
                </Show>
                <Show when={refusals().length > 0}>
                    <i
                        class="fa-solid fa-triangle-exclamation lan-peer-warn"
                        data-testid="lan-peer-key-warning-icon"
                        title="A message from this peer was refused: its key changed"
                    />
                </Show>
            </Button>
            <Show when={props.open}>
                <div class="lan-peer-body">
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">Address</span>
                        <span class="status-bar-popover-mono">
                            {props.peer.address}:{props.peer.port}
                        </span>
                    </div>
                    <Show when={platform()}>
                        <div class="status-bar-popover-row">
                            <span class="status-bar-popover-label">Runs</span>
                            <span>{platform()}</span>
                        </div>
                    </Show>
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">Agents</span>
                        <span>
                            {agentCount()}
                            <Show when={agentCount() > 0}>
                                {" "}
                                <Button tone="accent" density="compact" onClick={openSwarm}>
                                    Show in Swarm
                                </Button>
                            </Show>
                        </span>
                    </div>
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">Last seen</span>
                        <span>{lastSeenText(props.peer.last_seen, nowSecs())}</span>
                    </div>
                    <Show when={refusals().length > 0}>
                        <div class="lan-peer-key-warning" role="status" data-testid="lan-peer-key-warning">
                            <For each={refusals()}>
                                {(r) => (
                                    <div>
                                        A different key was refused for {r.agent} {r.refused.count} time
                                        {r.refused.count === 1 ? "" : "s"}; last{" "}
                                        {lastSeenText(r.refused.last_seen_at, nowSecs())}.
                                    </div>
                                )}
                            </For>
                            <Button tone="attention" density="compact" onClick={openPinnedSenders}>
                                Review in Settings › Devices › Pinned senders
                            </Button>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
}

/** The peers of the popover, with the pinned senders read once when the list appears. */
export function LanPeerList(props: { peers: readonly LanInstance[]; selfHost?: string }): JSX.Element {
    const [pins, setPins] = createSignal<readonly LanPinInfo[]>([]);
    onMount(() => {
        // The warning is a hint beside the list: a failed read leaves the rows without it.
        void (async () => {
            try {
                setPins((await RpcApi.PinsListCommand(TabRpcClient)).lan);
            } catch {
                setPins([]);
            }
        })();
    });
    // Kept here, by peer, because every event replaces the peer objects and a row's own state
    // would close under the reader.
    const [opened, setOpened] = createSignal<ReadonlySet<string>>(new Set());
    const toggle = (key: string) =>
        setOpened((prev) => {
            const next = new Set(prev);
            if (!next.delete(key)) next.add(key);
            return next;
        });
    const shared = createMemo(() => hostsWithSeveralInstances(props.peers, props.selfHost));
    return (
        <Index each={props.peers}>
            {(peer) => (
                <LanPeerRow
                    peer={peer()}
                    pins={pins()}
                    showChannel={showsChannel(peer(), shared())}
                    open={opened().has(peerKey(peer()))}
                    onToggle={() => toggle(peerKey(peer()))}
                />
            )}
        </Index>
    );
}
