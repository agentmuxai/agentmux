// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host popover panel's markup in parts (HostPopover.tsx): the host's identity and process, the LAN block (the
 * switch, its warnings and the peers), the MuxBus Cloud block, and the ports. Each reads the panel's props and its
 * state (host-popover-state.ts).
 */

import { getHostName } from "@/store/global";
import { devModeOn } from "@/app/store/dev-mode";
import { Show, type JSX } from "solid-js";
import { firewallMessage } from "./lan-indicator";
import { LanPeerList } from "./LanPeerRow";
import { MuxBusSignOutButton } from "@/app/view/accounts/muxbus-sign-out";
import { isLinux, isMacOS } from "@/util/platformutil";
import { Button, Switch } from "@/app/element/ui";
import type { HostInfo, HostPanelState, HostPopoverPanelProps } from "./host-popover-state";

type Props = { p: HostPopoverPanelProps };
type StateProps = Props & { t: HostPanelState };
/** The host's details; only rendered while there are some. */
const info = (p: HostPopoverPanelProps): HostInfo => p.hostInfo()!;

/** The host's name. */
export function HostIdentity(props: Props): JSX.Element {
    const p = props.p;
    return (
        <div class="status-bar-popover-row">
            <span style={{ "font-weight": "bold", "font-size": "1.05em" }}>{p.hostInfo()?.hostname ?? p.hostname}</span>
        </div>
    );
}

/** OS and IP. */
export function HostFacts(props: Props): JSX.Element {
    const p = props.p;
    return (
        <>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">OS</span>
                <span>{info(p).os}</span>
            </div>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">IP</span>
                <span class="status-bar-popover-mono">{info(p).localIp}</span>
            </div>
        </>
    );
}

/** The process id, and the data folder (the path is the link that opens it, with why it could not). */
export function ProcessRows(props: StateProps): JSX.Element {
    const { p, t } = props;
    return (
        <>
            <div class="status-bar-popover-divider" />
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">PID</span>
                <span class="status-bar-popover-mono">{info(p).pid}</span>
            </div>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">Data</span>
                {/* The path itself is the link: click opens it in the OS file
                    manager. The host resolves the directory from a closed
                    target, never from this path string.
                    Spec: SPEC_STATUSBAR_HOST_POPOVER_INSTANCE_AND_OPEN_DATA_DIR_2026_09_25.md §4 */}
                <Button
                    variant="link"
                    class="status-bar-popover-mono status-bar-popover-link"
                    style={{ "font-size": "0.85em" }}
                    data-tip={t.fileManagerLabel()}
                    onClick={() => void t.openDataDir()}
                >
                    {info(p).dataDir}
                </Button>
            </div>
            <Show when={t.openDirError()}>
                <div
                    class="status-bar-popover-row"
                    style={{
                        "padding-left": "12px",
                        "font-size": "0.85em",
                        color: "var(--warning-text-color)",
                    }}
                >
                    <span>⚠ {t.openDirError()}</span>
                </div>
            </Show>
        </>
    );
}

/** The LAN discovery switch, then what is wrong (the error, the firewall, being undiscoverable), then the peers. */
export function LanRows(props: Props): JSX.Element {
    const p = props.p;
    return (
        <>
            {/* Network — LAN discovery toggle.
                Spec: docs/specs/lan-discovery-toggle.md */}
            <div class="status-bar-popover-divider" />
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">LAN discovery</span>
                <span
                    class="status-bar-toggle"
                    data-tip={p.lanDiscoveryEnabled() ? "Disable" : "Enable (may prompt Windows Firewall)"}
                    style={{ "margin-left": "auto" }}
                >
                    <Switch
                        density="compact"
                        ariaLabel="LAN discovery"
                        checked={p.lanDiscoveryEnabled()}
                        onChange={(on) => p.onLanToggle(on)}
                    />
                </span>
            </div>
            <Show when={p.lanDiscoveryError()}>
                <div
                    class="status-bar-popover-row"
                    style={{
                        "padding-left": "12px",
                        "font-size": "0.85em",
                        color: "var(--warning-text-color)",
                    }}
                >
                    <span>⚠ {p.lanDiscoveryError()}</span>
                </div>
            </Show>
            <Show when={p.lanDiscoveryEnabled() && firewallMessage(p.lanFirewall()?.status)}>
                {(message) => (
                    <div
                        class="status-bar-popover-row"
                        data-testid="lan-firewall"
                        style={{
                            "padding-left": "12px",
                            "font-size": "0.85em",
                            color: "var(--warning-text-color)",
                        }}
                    >
                        <span>⚠ {message()}</span>
                    </div>
                )}
            </Show>
            <Show when={p.lanDiscoveryEnabled() && p.lanDiscoverability()?.state === "undiscoverable"}>
                <div
                    class="status-bar-popover-row"
                    data-testid="lan-undiscoverable"
                    style={{
                        "padding-left": "12px",
                        "font-size": "0.85em",
                        color: "var(--warning-text-color)",
                    }}
                >
                    <span>
                        ⚠ Other machines can't see this one, even though it may see them. Another program may be
                        using the mDNS port (UDP 5353). AgentMux keeps retrying on its own
                        {" "}({p.lanDiscoverability()?.rebuilds ?? 0} so far); turning LAN off and on retries at once.
                    </span>
                </div>
            </Show>
            <Show when={p.lanDiscoveryEnabled() && p.lanCount() > 0}>
                <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
                    <span style={{ color: "var(--accent-color)" }}>◆</span>
                    <span>{p.lanCount()} peer{p.lanCount() !== 1 ? "s" : ""}</span>
                </div>
                <LanPeerList peers={p.lanInstances()} selfHost={getHostName()} />
            </Show>
            <Show when={p.lanDiscoveryEnabled() && p.lanCount() === 0 && !p.lanDiscoveryError()}>
                <div
                    class="status-bar-popover-row"
                    style={{ "padding-left": "12px", opacity: "0.5", "font-size": "0.85em" }}
                >
                    <span>Searching for peers…</span>
                </div>
            </Show>
        </>
    );
}

/** The sign-in row: the account, or Sign in / Sign in again / Cancel. */
function MuxBusRow(props: StateProps): JSX.Element {
    const { t } = props;
    const muxbus = t.muxbus;
    return (
        <div class="status-bar-popover-row">
            <span class="status-bar-popover-label">MuxBus Cloud</span>
            <Show
                when={t.muxbusOk()}
                fallback={
                    <>
                        {/* A dead sign-in names the account it was for
                            (SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.5). */}
                        <Show when={t.signInAgain()}>
                            <span
                                class="status-bar-popover-mono"
                                style={{ "font-size": "0.85em", "margin-left": "auto" }}
                            >
                                {muxbus.status()?.email}
                            </span>
                        </Show>
                        <Button
                            tone="danger"
                            density="compact"
                            class="muxbus-login-chip muxbus-login-chip-signin"
                            style={{
                                "margin-left": t.signInAgain() ? "6px" : "auto",
                            }}
                            title={muxbus.loading() ? "Cancel sign-in" : undefined}
                            onClick={() => void (muxbus.loading() ? muxbus.cancel() : muxbus.connect())}
                        >
                            {muxbus.loading()
                                ? "Cancel"
                                : t.signInAgain()
                                  ? "Sign in again"
                                  : muxbus.status()?.connected
                                    ? "Expired — re-login"
                                    : "Sign in"}
                        </Button>
                    </>
                }
            >
                <span class="status-bar-popover-mono" style={{ "font-size": "0.85em" }}>{muxbus.status()?.email}</span>
            </Show>
        </div>
    );
}

/** The MuxBus Cloud block: the sign-in row, the keychain or keyring notice, Disconnect, and the error. */
export function MuxBusCloud(props: StateProps): JSX.Element {
    const { t } = props;
    const muxbus = t.muxbus;
    return (
        <Show when={t.showMuxbusCloud() && muxbus.isConfigured()}>
            <div class="status-bar-popover-divider" />
            <MuxBusRow {...props} />
            <Show when={isMacOS() && !t.muxbusOk()}>
                <div class="status-bar-popover-info-notice" role="status">
                    <span class="status-bar-popover-info-notice-icon" aria-hidden="true">
                        {"🔒"}
                    </span>
                    <span>
                        macOS will ask for Keychain access after you sign in — that's
                        AgentMux securely storing your session.
                    </span>
                </div>
            </Show>
            <Show when={isLinux() && !t.muxbusOk()}>
                <div class="status-bar-popover-info-notice" role="status">
                    <span class="status-bar-popover-info-notice-icon" aria-hidden="true">
                        {"🔒"}
                    </span>
                    <span>
                        Linux may ask for your keyring password after you sign in —
                        that's AgentMux securely storing your session.
                    </span>
                </div>
            </Show>
            <Show when={t.muxbusOk()}>
                <div class="status-bar-popover-row" style={{ "justify-content": "flex-end" }}>
                    <MuxBusSignOutButton
                        muxbus={muxbus}
                        class="muxbus-popover-disconnect-btn"
                        label="Disconnect"
                    />
                </div>
            </Show>
            <Show when={muxbus.error()}>
                <div class="status-bar-popover-row" style={{ "font-size": "0.85em", color: "var(--warning-text-color)" }}>
                    <span>⚠ {muxbus.error()}</span>
                </div>
            </Show>
        </Show>
    );
}

/** The ports (DevTools only in dev mode). */
export function PortRows(props: Props): JSX.Element {
    const p = props.p;
    return (
        <>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">IPC</span>
                <span class="status-bar-popover-mono">{info(p).ports.ipc}</span>
            </div>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">Backend</span>
                <span class="status-bar-popover-mono">{info(p).ports.web}</span>
            </div>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">WS</span>
                <span class="status-bar-popover-mono">{info(p).ports.ws}</span>
            </div>
            <Show when={devModeOn()}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">DevTools</span>
                    <span class="status-bar-popover-mono">{info(p).ports.devtools}</span>
                </div>
            </Show>
        </>
    );
}
