// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/store/global";
import { atoms } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo, useEffect, useRef, useState } from "react";

const BackendStatus = memo(() => {
    const backendStatus = useAtomValue(atoms.backendStatusAtom);
    const [popoverOpen, setPopoverOpen] = useState(false);
    const [backendInfo, setBackendInfo] = useState<{
        pid?: number;
        started_at?: string;
        web_endpoint?: string;
        version: string;
    } | null>(null);
    const popoverRef = useRef<HTMLDivElement>(null);

    let icon: string;
    let color: string;
    let label: string;
    let iconSpin = false;

    switch (backendStatus) {
        case "running":
            icon = "●";
            color = "var(--accent-color)";
            label = "Backend";
            break;
        case "connecting":
            icon = "◌";
            color = "var(--warning-color)";
            label = "Connecting…";
            iconSpin = true;
            break;
        case "crashed":
            icon = "●";
            color = "var(--error-color)";
            label = "Backend offline";
            break;
        default:
            return null;
    }

    const handleClick = async () => {
        if (popoverOpen) {
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

    useEffect(() => {
        if (!popoverOpen) return;
        const handleOutsideClick = (e: MouseEvent) => {
            if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
                setPopoverOpen(false);
            }
        };
        document.addEventListener("mousedown", handleOutsideClick);
        return () => document.removeEventListener("mousedown", handleOutsideClick);
    }, [popoverOpen]);

    const formatUptime = (startedAt: string): string => {
        const start = new Date(startedAt).getTime();
        const now = Date.now();
        const secs = Math.floor((now - start) / 1000);
        if (secs < 60) return `${secs}s`;
        const mins = Math.floor(secs / 60);
        if (mins < 60) return `${mins}m`;
        const hrs = Math.floor(mins / 60);
        return `${hrs}h ${mins % 60}m`;
    };

    return (
        <div style={{ position: "relative" }} ref={popoverRef}>
            <div
                className="status-bar-item clickable"
                title="Click for backend details"
                onClick={handleClick}
            >
                <span className={`status-icon${iconSpin ? " status-icon-spin" : ""}`} style={{ color }}>
                    {icon}
                </span>
                <span style={{ color }}>{label}</span>
            </div>
            {popoverOpen && (
                <div className="status-bar-popover">
                    <div className="status-bar-popover-row">
                        <span className="status-bar-popover-label">Status</span>
                        <span style={{ color }}>{backendStatus}</span>
                    </div>
                    {backendInfo?.pid && (
                        <div className="status-bar-popover-row">
                            <span className="status-bar-popover-label">PID</span>
                            <span>{backendInfo.pid}</span>
                        </div>
                    )}
                    {backendInfo?.started_at && (
                        <div className="status-bar-popover-row">
                            <span className="status-bar-popover-label">Uptime</span>
                            <span>{formatUptime(backendInfo.started_at)}</span>
                        </div>
                    )}
                    {backendInfo?.web_endpoint && (
                        <div className="status-bar-popover-row">
                            <span className="status-bar-popover-label">Endpoint</span>
                            <span className="status-bar-popover-mono">{backendInfo.web_endpoint}</span>
                        </div>
                    )}
                    {backendInfo?.version && (
                        <div className="status-bar-popover-row">
                            <span className="status-bar-popover-label">Version</span>
                            <span>{backendInfo.version}</span>
                        </div>
                    )}
                </div>
            )}
        </div>
    );
});

BackendStatus.displayName = "BackendStatus";

export { BackendStatus };
