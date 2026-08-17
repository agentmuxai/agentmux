// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, lanInstancesAtom, lanDiscoveryErrorAtom, setLanDiscoveryErrorAtom, settingsAtom } from "@/store/global";
import { invokeCommand } from "@/app/platform/ipc";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Accessor, createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { autoUpdate } from "@floating-ui/dom";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { computeMenuPosition } from "@/app/util/menu-position";
import { useMuxBusStatus, type MuxBusController } from "@/app/view/accounts/AgentMuxConnectPanel";
import QRCode from "qrcode";

type HostInfo = {
    hostname: string;
    os: string;
    localIp: string;
    instanceId: string;
    version: string;
    dataDir: string;
    hostType: string;
    pid: number;
    ports: {
        ipc: string;
        web: string;
        ws: string;
        devtools: string;
    };
};

interface HostPopoverPanelProps {
    anchorRect: DOMRect | null;
    onClose: () => void;
    hostname: string;
    hostInfo: Accessor<HostInfo | null>;
    lanInstances: Accessor<LanInstance[]>;
    lanCount: Accessor<number>;
    lanDiscoveryEnabled: Accessor<boolean>;
    lanDiscoveryError: Accessor<string | null>;
    onLanToggle: (enabled: boolean) => void;
    muxbus: MuxBusController;
    ref?: (el: HTMLDivElement) => void;
}

/**
 * The actual popover content, split out from `HostPopover` so it mounts
 * (and unmounts) only while open — `usePaneOverlay` and the floating-ui
 * position registration both need to run against the popover's OWN mount
 * lifecycle, not the always-mounted trigger's. Portaled to `document.body`
 * and airspace-clipped so it paints over any browser-pane HWND the status
 * bar overlaps, mirroring `TokenUsageIndicator` → `TokenBreakdownPopover`
 * (the canonical status-bar popover pattern).
 * Spec: SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md
 */
const HostPopoverPanel = (props: HostPopoverPanelProps): JSX.Element => {
    let rootRef: HTMLDivElement | undefined;

    // Airspace cut so the popover paints over any browser-pane HWND the
    // status bar overlaps — same primitive as TokenBreakdownPopover.
    usePaneOverlay(() => rootRef);

    // QR fallback for mobile pairing (Phase C). Off by default — only rendered
    // when the user explicitly clicks "Show QR code" while LAN discovery is on.
    // Local to the panel: closing/reopening the popover remounts this
    // component, which resets the signal for free — no manual reset needed.
    const [showQr, setShowQr] = createSignal(false);
    let qrCanvasRef: HTMLCanvasElement | undefined;

    // Builds the `agentmux://connect` deep link the mobile app scans to pair.
    // `token` is this instance's auth_key — the SAME value the backend already
    // broadcasts in plaintext in its mDNS TXT record whenever LAN discovery is
    // on (agentmux-srv/src/backend/lan_discovery.rs), and the same value this
    // frontend process already holds via the existing `get_auth_key` IPC
    // bootstrap call (getApi().getAuthKey(), used for every local RPC/WS call).
    // This helper only reads that already-cached value to build a string handed
    // straight to the QR renderer below — it is never logged and never sent
    // anywhere else.
    const connectUri = (): string | null => {
        const info = props.hostInfo();
        if (!info || !info.localIp || info.localIp === "127.0.0.1") return null;
        // Prefer the WS port (mobile's live connection); fall back to the web
        // port if WS somehow isn't available. Both endpoints are reported as
        // "127.0.0.1:<port>" — only the port number is meaningful for a LAN peer.
        const port = (info.ports.ws || info.ports.web || "").split(":").pop();
        const authKey = getApi()?.getAuthKey?.();
        if (!port || !authKey) return null;
        const params = new URLSearchParams({ host: info.localIp, port, token: authKey });
        return `agentmux://connect?${params.toString()}`;
    };

    // Render the QR code onto the canvas whenever the panel is opened (and
    // re-render if the underlying host info changes while it's open).
    createEffect(() => {
        if (!showQr()) return;
        const uri = connectUri();
        if (!uri || !qrCanvasRef) return;
        QRCode.toCanvas(qrCanvasRef, uri, { width: 176, margin: 1 }, (err) => {
            if (err) {
                // Never log `uri` here — it embeds the auth key.
                console.error("[HostPopover] failed to render pairing QR code:", err.message);
            }
        });
    });

    const muxbus = props.muxbus;
    const muxbusOk = () => {
        const s = muxbus.status();
        return !!s && s.connected && s.valid;
    };

    // Positioning routes through the shared primitive (mirrors
    // TokenBreakdownPopover): anchored to the hostname chip's rect,
    // placement top-end so the popover opens upward and right-aligns to
    // the chip — it lives in status-bar-right, near the window's right
    // edge, matching the pre-migration `.status-bar-popover.host-popover
    // { left: auto; right: 0; }` CSS override this replaces.
    const POPOVER_WIDTH = 320;
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        rootRef = el;
        props.ref?.(el);
        requestAnimationFrame(() => {
            const r = props.anchorRect;
            if (!r || !(el instanceof Element)) return;
            const update = async () => {
                const cur = props.anchorRect;
                if (!cur) return;
                const pos = await computeMenuPosition(
                    { anchor: cur, placement: "top-end", avoidNativePanes: false },
                    el,
                );
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            // anchorRect is a static DOMRect → virtual reference element.
            cleanupAutoUpdate = autoUpdate(
                { getBoundingClientRect: () => props.anchorRect ?? r },
                el,
                update,
            );
            // assertMenuInPaintableArea omitted: this popover uses usePaneOverlay
            // (airspace transparency cut-out), so intentional native-pane overlap
            // would produce a false-positive [menu-guard] warning.
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    return (
        <div
            ref={registerFloating}
            class="status-bar-popover host-popover"
            role="dialog"
            aria-label="Host info"
            data-pane-overlay
            style={{ ...floatingStyle(), width: `${POPOVER_WIDTH}px` }}
        >
            {/* Host Identity */}
            <div class="status-bar-popover-row">
                <span style={{ "font-weight": "bold", "font-size": "1.05em" }}>
                    {props.hostInfo()?.hostname ?? props.hostname}
                </span>
            </div>
            <Show when={props.hostInfo()}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">OS</span>
                    <span>{props.hostInfo()!.os}</span>
                </div>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">IP</span>
                    <span class="status-bar-popover-mono">{props.hostInfo()!.localIp}</span>
                </div>

                {/* Instance Info */}
                <div class="status-bar-popover-divider" />
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Instance</span>
                    <span>{props.hostInfo()!.instanceId}</span>
                </div>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">PID</span>
                    <span class="status-bar-popover-mono">{props.hostInfo()!.pid}</span>
                </div>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Data</span>
                    <span class="status-bar-popover-mono" style={{ "font-size": "0.85em", "max-width": "220px", "overflow": "hidden", "text-overflow": "ellipsis" }}>
                        {props.hostInfo()!.dataDir}
                    </span>
                </div>

                {/* Network — LAN discovery toggle.
                    Spec: specs/lan-discovery-toggle.md */}
                <div class="status-bar-popover-divider" />
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">LAN discovery</span>
                    <label
                        class="status-bar-toggle"
                        data-tip={props.lanDiscoveryEnabled() ? "Disable" : "Enable (may prompt Windows Firewall)"}
                        style={{ "margin-left": "auto" }}
                    >
                        <input
                            type="checkbox"
                            checked={props.lanDiscoveryEnabled()}
                            onChange={(e) =>
                                props.onLanToggle((e.target as HTMLInputElement).checked)
                            }
                        />
                    </label>
                </div>
                <Show when={props.lanDiscoveryError()}>
                    <div
                        class="status-bar-popover-row"
                        style={{
                            "padding-left": "12px",
                            "font-size": "0.85em",
                            color: "var(--warning-color, #d97706)",
                        }}
                    >
                        <span>⚠ {props.lanDiscoveryError()}</span>
                    </div>
                </Show>
                <Show when={props.lanDiscoveryEnabled() && props.lanCount() > 0}>
                    <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
                        <span style={{ color: "var(--accent-color)" }}>◆</span>
                        <span>{props.lanCount()} peer{props.lanCount() !== 1 ? "s" : ""}</span>
                    </div>
                    <For each={props.lanInstances()}>
                        {(inst: LanInstance) => (
                            <div class="status-bar-popover-row" style={{ "padding-left": "20px" }}>
                                <span style={{ opacity: "0.7" }}>{inst.hostname || inst.instance_id}</span>
                                <span class="status-bar-popover-mono" style={{ opacity: "0.5" }}>v{inst.version}</span>
                            </div>
                        )}
                    </For>
                </Show>
                <Show when={props.lanDiscoveryEnabled() && props.lanCount() === 0 && !props.lanDiscoveryError()}>
                    <div
                        class="status-bar-popover-row"
                        style={{ "padding-left": "12px", opacity: "0.5", "font-size": "0.85em" }}
                    >
                        <span>Searching for peers…</span>
                    </div>
                </Show>

                {/* QR fallback for mobile pairing when mDNS discovery
                    doesn't reach the phone (corporate/guest wifi,
                    VPN, different subnet, etc). Encodes the same
                    agentmux://connect deep link a peer would reach
                    via mDNS. Spec: Phase C, LAN-discovery reliability. */}
                <Show when={props.lanDiscoveryEnabled()}>
                    <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
                        <button
                            type="button"
                            class="status-bar-qr-toggle-btn"
                            onClick={() => setShowQr((v) => !v)}
                        >
                            {showQr() ? "Hide QR code" : "Show QR code"}
                        </button>
                    </div>
                    <Show when={showQr()}>
                        <div class="status-bar-qr-panel">
                            <canvas ref={qrCanvasRef} class="status-bar-qr-canvas" />
                            <div class="status-bar-qr-note">
                                For pairing the AgentMux mobile app on this network only —
                                don&apos;t share this code outside your local network.
                            </div>
                        </div>
                    </Show>
                </Show>

                {/* MuxBus Cloud */}
                <Show when={muxbus.isConfigured()}>
                    <div class="status-bar-popover-divider" />
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">MuxBus Cloud</span>
                        <Show
                            when={muxbus.status()?.connected && muxbus.status()?.valid}
                            fallback={
                                <button
                                    type="button"
                                    class="muxbus-login-chip muxbus-login-chip-signin"
                                    style={{ "margin-left": "auto", height: "auto", padding: "1px 8px" }}
                                    disabled={muxbus.loading()}
                                    onClick={() => void muxbus.connect()}
                                >
                                    {muxbus.loading() ? "●···" : muxbus.status()?.connected ? "Expired — re-login" : "Sign in"}
                                </button>
                            }
                        >
                            <span class="status-bar-popover-mono" style={{ "font-size": "0.85em" }}>{muxbus.status()?.email}</span>
                        </Show>
                    </div>
                    <Show when={muxbus.status()?.connected && muxbus.status()?.valid}>
                        <div class="status-bar-popover-row" style={{ "justify-content": "flex-end" }}>
                            <button
                                type="button"
                                class="muxbus-popover-disconnect-btn"
                                disabled={muxbus.loading()}
                                onClick={() => void muxbus.disconnect()}
                            >
                                {muxbus.loading() ? "Disconnecting…" : "Disconnect"}
                            </button>
                        </div>
                    </Show>
                    <Show when={muxbus.error()}>
                        <div class="status-bar-popover-row" style={{ "font-size": "0.85em", color: "var(--warning-color, #d97706)" }}>
                            <span>⚠ {muxbus.error()}</span>
                        </div>
                    </Show>
                </Show>

                <div class="status-bar-popover-divider" />

                {/* Ports */}
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">IPC</span>
                    <span class="status-bar-popover-mono">{props.hostInfo()!.ports.ipc}</span>
                </div>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Backend</span>
                    <span class="status-bar-popover-mono">{props.hostInfo()!.ports.web}</span>
                </div>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">WS</span>
                    <span class="status-bar-popover-mono">{props.hostInfo()!.ports.ws}</span>
                </div>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">DevTools</span>
                    <span class="status-bar-popover-mono">{props.hostInfo()!.ports.devtools}</span>
                </div>
            </Show>
        </div>
    );
};

HostPopoverPanel.displayName = "HostPopoverPanel";

const HostPopover = (): JSX.Element => {
    const hostname = getApi().getHostName();
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [hostInfo, setHostInfo] = createSignal<HostInfo | null>(null);
    const [anchorRect, setAnchorRect] = createSignal<DOMRect | null>(null);
    let triggerRef: HTMLDivElement | undefined;
    let popoverRef: HTMLDivElement | undefined;
    const muxbus = useMuxBusStatus();

    // Keep the trigger's muxbus dot current without requiring the popover
    // to ever be opened — a dead session must be visible at a glance (the
    // popover-only Sign in state let the 0.55.8 rollout sit with WAN jekt
    // delivery silently dead for hours).
    onMount(() => {
        void muxbus.refresh();
        const timer = window.setInterval(() => void muxbus.refresh(), 60_000);
        onCleanup(() => window.clearInterval(timer));
    });
    const muxbusOk = () => {
        const s = muxbus.status();
        return !!s && s.connected && s.valid;
    };

    const lanInstances = lanInstancesAtom;
    const lanCount = () => lanInstances().length;
    const lanDiscoveryEnabled = () => !!settingsAtom()?.["network:lan_discovery"];
    const lanDiscoveryError = lanDiscoveryErrorAtom;

    // Toggle the network:lan_discovery setting. The backend's setconfig handler
    // calls LanDiscoveryController.apply, which starts/stops the mDNS daemon
    // live — no restart. On Windows, the first enable triggers the firewall
    // prompt; if the user clicks Block, a "laninstances:error" event flows back
    // and surfaces in the panel.
    // Spec: specs/lan-discovery-toggle.md
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
            const info = await invokeCommand<HostInfo>("get_host_info", {});
            setHostInfo(info);
        } catch {
            // Fallback for Tauri (doesn't have get_host_info yet)
            setHostInfo(null);
        }
        void muxbus.refresh();
        if (triggerRef) setAnchorRect(triggerRef.getBoundingClientRect());
        setPopoverOpen(true);
    };

    // Close on outside click — ignores clicks on the trigger or inside the
    // portaled popover. Dual-ref pattern (mirrors TokenUsageIndicator):
    // now that the popover is portaled to document.body instead of nested
    // under the trigger, a single containment check against the trigger
    // alone would treat every click inside the popover as "outside."
    createEffect(() => {
        if (!popoverOpen()) return;
        const handleOutsideClick = (e: MouseEvent) => {
            const t = e.target as Node;
            if (triggerRef?.contains(t) || popoverRef?.contains(t)) return;
            setPopoverOpen(false);
        };
        document.addEventListener("mousedown", handleOutsideClick);
        onCleanup(() => document.removeEventListener("mousedown", handleOutsideClick));
    });

    return (
        <Show when={hostname && hostname !== "unknown"}>
            <div
                ref={(el) => { triggerRef = el; }}
                class="status-bar-item clickable"
                data-tip="Host info, click for details"
                aria-label="Host info"
                onClick={handleClick}
            >
                <span class="status-hostname">
                    {hostname}
                </span>
                <Show when={lanCount() > 0}>
                    <span style={{ color: "var(--accent-color)", "margin-left": "4px" }}>{"◆"}</span>
                </Show>
                <Show when={muxbus.isConfigured() && muxbus.status() !== null}>
                    <span
                        class="status-muxbus-dot"
                        classList={{ "status-muxbus-dot--ok": muxbusOk() }}
                        data-tip={muxbusOk() ? "MuxBus connected" : "MuxBus not connected — click for details and sign in"}
                        aria-label={muxbusOk() ? "MuxBus connected" : "MuxBus not connected"}
                    />
                </Show>
            </div>
            <Show when={popoverOpen()}>
                <Portal>
                    <HostPopoverPanel
                        anchorRect={anchorRect()}
                        onClose={() => setPopoverOpen(false)}
                        hostname={hostname}
                        hostInfo={hostInfo}
                        lanInstances={lanInstances}
                        lanCount={lanCount}
                        lanDiscoveryEnabled={lanDiscoveryEnabled}
                        lanDiscoveryError={lanDiscoveryError}
                        onLanToggle={(enabled) => void handleLanToggle(enabled)}
                        muxbus={muxbus}
                        ref={(el) => { popoverRef = el; }}
                    />
                </Portal>
            </Show>
        </Show>
    );
};

HostPopover.displayName = "HostPopover";

export { HostPopover };
