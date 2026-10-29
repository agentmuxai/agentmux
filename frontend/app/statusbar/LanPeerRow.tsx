// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One discovered LAN peer in the network pill's popover: its name and version on the line, and,
// opened, where it is, what it runs, who is on it and whether a message from its agents was refused
// for a key that changed. Re-pinning stays in Settings > Devices > Pinned senders, beside its
// explanation (docs/specs/SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §5).

import { For, Show, createMemo, createSignal, onMount, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { LanInstance } from "@/types/rpc/LanInstance";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import { lastSeenText, osAndChannel, peerKeyRefusals } from "./lan-peer";

const openPinnedSenders = () =>
    void openOrFocusPaneByView(
        "settings",
        { meta: { view: "settings", "settings:section": "devices" } },
        { "settings:section": "devices" }
    );

const openSwarm = () => void openOrFocusPaneByView("swarm");

export function LanPeerRow(props: { peer: LanInstance; pins: readonly LanPinInfo[] }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const tick = useTick(1000);
    const nowSecs = createMemo(() => (tick(), Math.floor(Date.now() / 1000)));
    const refusals = createMemo(() => peerKeyRefusals(props.peer, props.pins));
    const name = () => props.peer.hostname || props.peer.instance_id || "resolving…";
    const platform = () => osAndChannel(props.peer);
    const agentCount = () => props.peer.agents.length;
    return (
        <div class="lan-peer" data-testid="lan-peer">
            <button type="button" class="lan-peer-head" aria-expanded={open()} onClick={() => setOpen(!open())}>
                <i class={`fa-solid ${open() ? "fa-chevron-down" : "fa-chevron-right"} lan-peer-chevron`} />
                <span class="lan-peer-name">{name()}</span>
                <Show when={props.peer.version}>
                    <span class="status-bar-popover-mono lan-peer-version">v{props.peer.version}</span>
                </Show>
                <Show when={refusals().length > 0}>
                    <i
                        class="fa-solid fa-triangle-exclamation lan-peer-warn"
                        data-testid="lan-peer-key-warning-icon"
                        title="A message from this peer was refused: its key changed"
                    />
                </Show>
            </button>
            <Show when={open()}>
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
                                <button type="button" class="lan-peer-link" onClick={openSwarm}>
                                    Show in Swarm
                                </button>
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
                            <button type="button" class="lan-peer-link" onClick={openPinnedSenders}>
                                Review in Settings › Devices › Pinned senders
                            </button>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
}

/** The peers of the popover, with the pinned senders read once when the list appears. */
export function LanPeerList(props: { peers: readonly LanInstance[] }): JSX.Element {
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
    return <For each={props.peers}>{(peer) => <LanPeerRow peer={peer} pins={pins()} />}</For>;
}
