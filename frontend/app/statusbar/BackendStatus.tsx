// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, backendDeathInfoAtom, getApi, setBackendStatusAtom, termRendererAtom } from "@/store/global";
import { setRestartInProgress } from "@/store/backendStatus";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getGpuInfo } from "@/util/gpuutil";
import { Accessor, createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { formatUptime, resolveUptimeSecs } from "./backend-uptime";

function gpuColor(c: ReturnType<typeof getGpuInfo>["classification"]): string {
    switch (c) {
        case "hardware":
            return "var(--accent-color)";
        case "software":
            return "var(--warning-color)";
        default:
            return "var(--error-color)";
    }
}

type BackendInfo = {
    pid?: number;
    started_at?: string;
    web_endpoint?: string;
    version: string;
    pending_migrations?: number;
};

interface BackendStatusPanelProps {
    /** The status bar trigger. */
    anchor: PopoverAnchor;
    onClose: () => void;
    backendInfo: Accessor<BackendInfo | null>;
    startedAt: Accessor<number | null>;
    uptimeSecs: Accessor<number>;
    restarting: Accessor<boolean>;
    onRestart: () => void;
    gpu: ReturnType<typeof getGpuInfo>;
}

/**
 * The actual popover content, split out from `BackendStatus` so it mounts
 * (and unmounts) only while open: AnchoredPopover's positioning, dismiss
 * and airspace cut run for the panel's own lifetime, not the always-mounted
 * trigger's. Spec: SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md
 */
const BackendStatusPanel = (props: BackendStatusPanelProps): JSX.Element => {
    const backendStatus = atoms.backendStatusAtom;
    const backendInfo = props.backendInfo;

    const color = () => {
        switch (backendStatus()) {
            case "running": return "var(--accent-color)";
            case "connecting": return "var(--warning-color)";
            case "crashed": return "var(--error-color)";
            default: return null;
        }
    };

    // Opens upward, left-aligned to the status dot. Positioning, chrome zoom, dismiss
    // and the airspace cut are AnchoredPopover's.
    const POPOVER_WIDTH = 260;

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-start"
            onDismiss={props.onClose}
            class="status-bar-popover"
            role="dialog"
            aria-label="Backend status"
            style={{ width: `${POPOVER_WIDTH}px` }}
        >
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">Status</span>
                <span style={{ color: color() }}>{backendStatus()}</span>
            </div>
            <Show when={backendInfo()?.pid}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">PID</span>
                    <span>{backendInfo().pid}</span>
                </div>
            </Show>
            <Show when={props.startedAt() != null}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Uptime</span>
                    <span>{formatUptime(props.uptimeSecs())}</span>
                </div>
            </Show>
            <Show when={backendInfo()?.web_endpoint}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Endpoint</span>
                    <span class="status-bar-popover-mono">{backendInfo().web_endpoint}</span>
                </div>
            </Show>
            {/* No separate "Version" row here — the srv version always
                matches the app version already shown in the Instance panel
                (v{about().version} in the status bar's bottom-right chip),
                since both ship from the same workspace release. */}
            <Show when={(backendInfo()?.pending_migrations ?? 0) > 0}>
                <div class="status-bar-popover-divider" />
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label" style={{ color: "var(--warning-color)" }}>
                        Migrations
                    </span>
                    <span style={{ color: "var(--warning-color)" }}>
                        {backendInfo()!.pending_migrations} pending
                    </span>
                </div>
                <div class="status-bar-popover-row">
                    <button
                        type="button"
                        class="status-bar-restart-btn"
                        onClick={() => {
                            props.onClose();
                            window.dispatchEvent(new CustomEvent("agentmux:open-version-panel"));
                        }}
                    >
                        Open Maintenance ↗
                    </button>
                </div>
            </Show>
            {/* GPU / WebGL rendering — enabled/disabled + driver info.
                Surfaces the silent DOM-renderer fallback when the GPU
                process is unavailable. */}
            <div class="status-bar-popover-divider" />
            <div class="status-bar-popover-section-title">GPU</div>
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">Status</span>
                <span style={{ color: gpuColor(props.gpu.classification) }}>
                    {props.gpu.classification === "unavailable"
                        ? "Disabled"
                        : props.gpu.classification === "software"
                            ? "Enabled (software)"
                            : "Enabled (hardware)"}
                </span>
            </div>
            <Show when={props.gpu.webgl}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">WebGL</span>
                    <span>{props.gpu.webgl2 ? "2.0" : "1.0"}</span>
                </div>
            </Show>
            <Show when={props.gpu.renderer}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Driver</span>
                    <span class="status-bar-popover-mono">{props.gpu.renderer}</span>
                </div>
            </Show>
            <Show when={props.gpu.vendor}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Vendor</span>
                    <span class="status-bar-popover-mono">{props.gpu.vendor}</span>
                </div>
            </Show>
            <Show when={termRendererAtom() != null}>
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Terminal</span>
                    <span style={{ color: termRendererAtom() === "webgl" ? "var(--accent-color)" : "var(--warning-color)" }}>
                        {termRendererAtom() === "webgl" ? "WebGL (GPU)" : "DOM (software)"}
                    </span>
                </div>
            </Show>
            <Show when={backendStatus() === "crashed" && backendDeathInfoAtom() != null}>
                <div class="status-bar-popover-divider" />
                <div class="status-bar-popover-row">
                    <span class="status-bar-popover-label">Died at</span>
                    <span>{new Date(backendDeathInfoAtom()!.died_at).toLocaleTimeString()}</span>
                </div>
                <Show when={backendDeathInfoAtom()!.uptime_secs != null}>
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">Was up</span>
                        <span>{formatUptime(backendDeathInfoAtom()!.uptime_secs!)}</span>
                    </div>
                </Show>
                <Show when={backendDeathInfoAtom()!.code != null}>
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">Exit code</span>
                        <span class="status-bar-popover-mono">{backendDeathInfoAtom()!.code}</span>
                    </div>
                </Show>
                <Show when={backendDeathInfoAtom()!.signal != null}>
                    <div class="status-bar-popover-row">
                        <span class="status-bar-popover-label">Signal</span>
                        <span class="status-bar-popover-mono">{backendDeathInfoAtom()!.signal}</span>
                    </div>
                </Show>
                <div class="status-bar-popover-divider" />
                <div class="status-bar-popover-row">
                    <button
                        class="status-bar-restart-btn"
                        disabled={props.restarting()}
                        onClick={props.onRestart}
                    >
                        {props.restarting() ? "Restarting…" : "Restart Backend"}
                    </button>
                </div>
            </Show>
        </AnchoredPopover>
    );
};

BackendStatusPanel.displayName = "BackendStatusPanel";

const BackendStatus = (): JSX.Element => {
    const backendStatus = atoms.backendStatusAtom;
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [restarting, setRestarting] = createSignal(false);

    const handleRestart = () => {
        setRestarting(true);
        setRestartInProgress(true); // suppress backend-terminated → crashed during restart
        setBackendStatusAtom("connecting");
        setPopoverOpen(false);
        getApi().restartBackend().catch((e: unknown) => {
            console.error("[BackendStatus] restart failed:", e);
            setRestartInProgress(false); // clear flag — restart failed, allow future crash events
            setBackendStatusAtom("crashed");
        }).finally(() => {
            setRestarting(false);
        });
    };

    const [startedAt, setStartedAt] = createSignal<number | null>(null);
    const [uptimeSecs, setUptimeSecs] = createSignal(0);
    const [backendInfo, setBackendInfo] = createSignal<BackendInfo | null>(null);
    let triggerRef: HTMLDivElement | undefined;
    const gpu = getGpuInfo(); // WebGL/GPU capability — static for the renderer process

    // Fetch started_at when backend becomes running
    createEffect(() => {
        const status = backendStatus();
        if (status === "running" && startedAt() == null) {
            getApi().getBackendInfo().then((info) => {
                setBackendInfo(info);
                if (info?.started_at) {
                    setStartedAt(new Date(info.started_at).getTime());
                }
            }).catch(() => {});
        }
    });

    // Drive uptime from the sysinfo event so all windows update in sync — every
    // window receives the same tick and shows the same integer, eliminating
    // phase drift.
    //
    // The value itself comes from srv's MONOTONIC `uptime_secs`, not from
    // subtracting the event's wall-clock `ts` from the host's `started_at`
    // stamp. Those are two independent wall-clock reads spanning the backend's
    // whole lifetime, so any backwards clock step (NTP correction, manual set,
    // VM resume) made the difference negative and left it that way until the
    // next restart. See `backend-uptime.ts` for the live 2081 -> 2026 case this
    // fixes. `ts` is still passed as a clamped fallback for a payload that
    // predates the field.
    onMount(() => {
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: "local",
            handler: (event) => {
                const data = (event as MuxEvent)?.data;
                const next = resolveUptimeSecs(data?.uptime_secs, data?.ts, startedAt());
                if (next != null) {
                    setUptimeSecs(next);
                }
            },
        });
        onCleanup(() => unsub?.());
    });

    const icon = () => {
        switch (backendStatus()) {
            case "running": return "●";
            case "connecting": return "◌";
            case "crashed": return "●";
            default: return null;
        }
    };

    const color = () => {
        switch (backendStatus()) {
            case "running": return "var(--accent-color)";
            case "connecting": return "var(--warning-color)";
            case "crashed": return "var(--error-color)";
            default: return null;
        }
    };

    const iconSpin = () => backendStatus() === "connecting";

    const handleClick = async () => {
        if (popoverOpen()) {
            setPopoverOpen(false);
            return;
        }
        try {
            const info = await getApi().getBackendInfo();
            setBackendInfo(info);
        } catch {
            setBackendInfo(null);
        }
        setPopoverOpen(true);
    };


    return (
        <Show when={backendStatus() !== null && icon() !== null}>
            <div
                ref={(el) => { triggerRef = el; }}
                class="status-bar-item clickable"
                data-tip="Backend status, click for details"
                aria-label="Backend status"
                onClick={handleClick}
            >
                <span class={`status-icon${iconSpin() ? " status-icon-spin" : ""}`} style={{ color: color() }}>
                    {icon()}
                </span>
                <Show when={backendStatus() === "running" && startedAt() != null}>
                    <span
                        class="stat-mono stat-uptime"
                        style={{ "min-width": uptimeSecs() < 3600 ? "5ch" : uptimeSecs() < 86400 ? "8ch" : "12ch" }}
                    >{formatUptime(uptimeSecs())}</span>
                </Show>
                <Show when={backendStatus() === "connecting"}>
                    <span style={{ color: color() }}>Connecting…</span>
                </Show>
                <Show when={backendStatus() === "crashed"}>
                    <span style={{ color: color() }}>Offline</span>
                </Show>
            </div>
            <Show when={popoverOpen()}>
                    <BackendStatusPanel
                        anchor={triggerRef}
                        onClose={() => setPopoverOpen(false)}
                        backendInfo={backendInfo}
                        startedAt={startedAt}
                        uptimeSecs={uptimeSecs}
                        restarting={restarting}
                        onRestart={handleRestart}
                        gpu={gpu}
                    />
            </Show>
        </Show>
    );
};

BackendStatus.displayName = "BackendStatus";

export { BackendStatus };
