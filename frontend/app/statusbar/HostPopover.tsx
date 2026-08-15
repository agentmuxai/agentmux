// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, lanInstancesAtom, lanDiscoveryErrorAtom, setLanDiscoveryErrorAtom, settingsAtom } from "@/store/global";
import { invokeCommand } from "@/app/platform/ipc";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";
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

const HostPopover = (): JSX.Element => {
    const hostname = getApi().getHostName();
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [hostInfo, setHostInfo] = createSignal<HostInfo | null>(null);
    let popoverRef!: HTMLDivElement;
    const muxbus = useMuxBusStatus();

    const lanInstances = lanInstancesAtom;
    const lanCount = () => lanInstances().length;
    const lanDiscoveryEnabled = () => !!settingsAtom()?.["network:lan_discovery"];
    const lanDiscoveryError = lanDiscoveryErrorAtom;

    // QR fallback for mobile pairing (Phase C). Off by default — only rendered
    // when the user explicitly clicks "Show QR code" while LAN discovery is on.
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
        const info = hostInfo();
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
            setShowQr(false);
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
        setPopoverOpen(true);
    };

    createEffect(() => {
        if (!popoverOpen()) return;
        const handleOutsideClick = (e: MouseEvent) => {
            if (popoverRef && !popoverRef.contains(e.target as Node)) {
                setPopoverOpen(false);
                setShowQr(false);
            }
        };
        document.addEventListener("mousedown", handleOutsideClick);
        onCleanup(() => document.removeEventListener("mousedown", handleOutsideClick));
    });

    return (
        <Show when={hostname && hostname !== "unknown"}>
            <div style={{ position: "relative" }} ref={popoverRef}>
                <div
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
                </div>
                <Show when={popoverOpen()}>
                    <div class="status-bar-popover host-popover">
                        {/* Host Identity */}
                        <div class="status-bar-popover-row">
                            <span style={{ "font-weight": "bold", "font-size": "1.05em" }}>
                                {hostInfo()?.hostname ?? hostname}
                            </span>
                        </div>
                        <Show when={hostInfo()}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">OS</span>
                                <span>{hostInfo()!.os}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">IP</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.localIp}</span>
                            </div>

                            {/* Instance Info */}
                            <div class="status-bar-popover-divider" />
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Instance</span>
                                <span>{hostInfo()!.instanceId}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">PID</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.pid}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Data</span>
                                <span class="status-bar-popover-mono" style={{ "font-size": "0.85em", "max-width": "220px", "overflow": "hidden", "text-overflow": "ellipsis" }}>
                                    {hostInfo()!.dataDir}
                                </span>
                            </div>

                            {/* Network — LAN discovery toggle.
                                Spec: specs/lan-discovery-toggle.md */}
                            <div class="status-bar-popover-divider" />
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">LAN discovery</span>
                                <label
                                    class="status-bar-toggle"
                                    data-tip={lanDiscoveryEnabled() ? "Disable" : "Enable (may prompt Windows Firewall)"}
                                    style={{ "margin-left": "auto" }}
                                >
                                    <input
                                        type="checkbox"
                                        checked={lanDiscoveryEnabled()}
                                        onChange={(e) =>
                                            void handleLanToggle((e.target as HTMLInputElement).checked)
                                        }
                                    />
                                </label>
                            </div>
                            <Show when={lanDiscoveryError()}>
                                <div
                                    class="status-bar-popover-row"
                                    style={{
                                        "padding-left": "12px",
                                        "font-size": "0.85em",
                                        color: "var(--warning-color, #d97706)",
                                    }}
                                >
                                    <span>⚠ {lanDiscoveryError()}</span>
                                </div>
                            </Show>
                            <Show when={lanDiscoveryEnabled() && lanCount() > 0}>
                                <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
                                    <span style={{ color: "var(--accent-color)" }}>◆</span>
                                    <span>{lanCount()} peer{lanCount() !== 1 ? "s" : ""}</span>
                                </div>
                                <For each={lanInstances()}>
                                    {(inst: LanInstance) => (
                                        <div class="status-bar-popover-row" style={{ "padding-left": "20px" }}>
                                            <span style={{ opacity: "0.7" }}>{inst.hostname || inst.instance_id}</span>
                                            <span class="status-bar-popover-mono" style={{ opacity: "0.5" }}>v{inst.version}</span>
                                        </div>
                                    )}
                                </For>
                            </Show>
                            <Show when={lanDiscoveryEnabled() && lanCount() === 0 && !lanDiscoveryError()}>
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
                            <Show when={lanDiscoveryEnabled()}>
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
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.ipc}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Backend</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.web}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">WS</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.ws}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">DevTools</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.devtools}</span>
                            </div>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

HostPopover.displayName = "HostPopover";

export { HostPopover };
