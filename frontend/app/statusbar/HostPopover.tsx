// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    getApi,
    getHostName,
    lanDiscoverabilityAtom,
    lanFirewallAtom,
    lanInstancesAtom,
    lanDiscoveryErrorAtom,
    setLanDiscoveryErrorAtom,
    settingsAtom,
} from "@/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createEffect, createSignal, onCleanup, Show, untrack, type JSX } from "solid-js";
import { resolveLanIndicator } from "./lan-indicator";
import { AnchoredPopover } from "@/app/element/anchored-popover";
import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";
import { isMuxBusSessionOk, muxbusNeedsSignInAgain } from "@/app/view/accounts/muxbus-session";
import { PairDevicePanel } from "./PairDevicePanel";
import { HostFacts, HostIdentity, LanRows, MuxBusCloud, PortRows, ProcessRows } from "./host-popover-parts";
import { createHostPanel, type HostInfo, type HostPopoverPanelProps } from "./host-popover-state";

/** Whether the host's reported name is srv's machine (names compare case-insensitively). */
export function sameMachine(hostName: string | undefined, srvHostName: string | undefined): boolean {
    return !!hostName && !!srvHostName && hostName.toLowerCase() === srvHostName.toLowerCase();
}

/**
 * The actual popover content, split out from `HostPopover` so it mounts
 * (and unmounts) only while open: AnchoredPopover's positioning, dismiss
 * and airspace cut run for the panel's own lifetime, not the always-mounted
 * trigger's. Spec: SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md
 */
const HostPopoverPanel = (props: HostPopoverPanelProps): JSX.Element => {
    const t = createHostPanel(props);

    // Opens upward, right-aligned to the hostname chip. Positioning, chrome zoom, dismiss
    // and the airspace cut are AnchoredPopover's.
    const POPOVER_WIDTH = 320;

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class="status-bar-popover host-popover"
            fitHeight
            role="dialog"
            aria-label="Host info"
            style={{ width: `${POPOVER_WIDTH}px` }}
        >
            <HostIdentity p={props} />
            <Show when={props.hostInfo()}>
                <HostFacts p={props} />
                <ProcessRows p={props} t={t} />
                <LanRows p={props} />

                {/* Pairing a device with this computer's viewer: a QR with a
                    one-time code and the certificate's fingerprint, never the
                    instance key (agentmux-mobile's
                    SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §13.2). It
                    replaces the old `agentmux://connect` QR, which carried the
                    instance's full auth key. */}
                <PairDevicePanel lanDiscoveryEnabled={props.lanDiscoveryEnabled} />

                <MuxBusCloud p={props} t={t} />

                <div class="status-bar-popover-divider" />

                <PortRows p={props} />
            </Show>
        </AnchoredPopover>
    );
};

HostPopoverPanel.displayName = "HostPopoverPanel";

const HostPopover = (): JSX.Element => {
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [hostInfo, setHostInfo] = createSignal<HostInfo | null>(null);
    let triggerRef: HTMLDivElement | undefined;
    const muxbus = useMuxBusStatus();

    // `statusbar:showmuxbuscloud`: only an explicit false hides MuxBus Cloud (the
    // dot and the popover's sign-in block). LAN is separate and never hidden.
    // SPEC_STATUSBAR_HIDE_MUXBUS_SETTING_2026_10_08.md.
    const showMuxbusCloud = () => settingsAtom()?.["statusbar:showmuxbuscloud"] !== false;

    // Keep the trigger's muxbus dot current without requiring the popover
    // to ever be opened — a dead session must be visible at a glance (the
    // popover-only Sign in state let the 0.55.8 rollout sit with WAN jekt
    // delivery silently dead for hours). Not polled while the user has hidden
    // MuxBus Cloud; turning it back on refreshes straight away.
    createEffect(() => {
        if (!showMuxbusCloud()) return;
        // untrack: refresh() reads its own signals, which must not re-run this.
        untrack(() => void muxbus.refresh());
        const timer = window.setInterval(() => void muxbus.refresh(), 60_000);
        onCleanup(() => window.clearInterval(timer));
    });
    const muxbusOk = () => isMuxBusSessionOk(muxbus.status());
    const muxbusTip = () =>
        muxbusOk()
            ? "MuxBus connected"
            : muxbusNeedsSignInAgain(muxbus.status())
              ? "MuxBus sign-in no longer works — click to sign in again"
              : "MuxBus not connected — click for details and sign in";

    const lanInstances = lanInstancesAtom;
    const lanCount = () => lanInstances().length;
    const lanDiscoveryEnabled = () => !!settingsAtom()?.["network:lan_discovery"];
    const lanDiscoveryError = lanDiscoveryErrorAtom;

    // Glyph + label + state in one place (`lan-indicator.ts`), so the tooltip
    // a user reads can never disagree with the glyph they see.
    const lanIndicator = () =>
        resolveLanIndicator({
            enabled: lanDiscoveryEnabled(),
            peerCount: lanCount(),
            error: lanDiscoveryError(),
            discoverability: lanDiscoverabilityAtom()?.state,
            firewall: lanFirewallAtom()?.status,
        });

    // Toggle the network:lan_discovery setting. The backend's setconfig handler
    // calls LanDiscoveryController.apply, which starts/stops the mDNS daemon
    // live — no restart. On Windows, the first enable triggers the firewall
    // prompt; if the user clicks Block, a "laninstances:error" event flows back
    // and surfaces in the panel.
    // Spec: docs/specs/lan-discovery-toggle.md
    const handleLanToggle = async (enabled: boolean) => {
        // Optimistic clear of any prior error; backend will resend if it still
        // can't start the daemon.
        setLanDiscoveryErrorAtom(null);
        try {
            await RpcApi.SetConfigCommand(TabRpcClient, { "network:lan_discovery": enabled } as any);
        } catch (e) {
            setLanDiscoveryErrorAtom(`Failed to update setting: ${e}`);
        }
    };

    const handleClick = async () => {
        if (popoverOpen()) {
            setPopoverOpen(false);
            return;
        }
        try {
            const info = (await getApi().getHostInfo()) as unknown as HostInfo;
            // The chip names srv's machine. The host's own details (its OS, IP,
            // PID, data folder) describe that machine only when the host runs
            // on it; for a host elsewhere they'd silently describe another one.
            setHostInfo(sameMachine(info?.hostname, getHostName()) ? info : null);
        } catch {
            // Fallback for a host build without get_host_info
            setHostInfo(null);
        }
        if (showMuxbusCloud()) void muxbus.refresh();
        setPopoverOpen(true);
    };

    return (
        <Show when={getHostName() && getHostName() !== "unknown"}>
            <div
                ref={(el) => { triggerRef = el; }}
                class="status-bar-item clickable"
                data-tip="Host info, click for details"
                aria-label="Host info"
                onClick={handleClick}
            >
                <span class="status-hostname">
                    {getHostName()}
                </span>
                {/* Three states, always one of them rendered — see
                    docs/retro/retro-lan-diamond-vanished-after-self-peer-fix-2026-09-06.md.
                    Peer-count alone used to drive this, so the indicator
                    disappeared entirely once #3025 stopped an instance
                    discovering ITSELF as a phantom peer: a lone instance with
                    discovery genuinely on then looked identical to one with it
                    off. Enabled-but-idle and off are distinct facts and each
                    now has its own glyph. */}
                <span
                    class="status-lan-diamond"
                    classList={{ [`status-lan-diamond--${lanIndicator().state}`]: true }}
                    data-tip={lanIndicator().label}
                    aria-label={lanIndicator().label}
                >
                    {lanIndicator().glyph}
                </span>
                <Show when={showMuxbusCloud() && muxbus.isConfigured() && muxbus.status() !== null}>
                    <span
                        class="status-muxbus-dot"
                        classList={{ "status-muxbus-dot--ok": muxbusOk() }}
                        data-tip={muxbusTip()}
                        aria-label={muxbusOk() ? "MuxBus connected" : "MuxBus not connected"}
                    />
                </Show>
            </div>
            <Show when={popoverOpen()}>
                <HostPopoverPanel
                    anchor={triggerRef}
                    onClose={() => setPopoverOpen(false)}
                    hostname={getHostName()}
                    hostInfo={hostInfo}
                    lanInstances={lanInstances}
                    lanCount={lanCount}
                    lanDiscoveryEnabled={lanDiscoveryEnabled}
                    lanDiscoveryError={lanDiscoveryError}
                    lanDiscoverability={lanDiscoverabilityAtom}
                    lanFirewall={lanFirewallAtom}
                    onLanToggle={(enabled) => void handleLanToggle(enabled)}
                    muxbus={muxbus}
                    showMuxbusCloud={showMuxbusCloud}
                />
            </Show>
        </Show>
    );
};

HostPopover.displayName = "HostPopover";

export { HostPopover, HostPopoverPanel };
