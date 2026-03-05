// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, windowCountAtom, windowInstanceNumAtom } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { ConnectionStatus } from "./ConnectionStatus";
import { UpdateStatus } from "./UpdateStatus";
import "./StatusBar.scss";

const StatusBar = memo(() => {
    const version = getApi().getAboutModalDetails()?.version ?? "";
    const instanceNum = useAtomValue(windowInstanceNumAtom);
    const windowCount = useAtomValue(windowCountAtom);

    const handleNewWindow = async () => {
        try {
            await getApi().openNewWindow();
        } catch (error) {
            console.error("[StatusBar] Failed to open new window:", error);
        }
    };

    return (
        <div className="status-bar">
            <div className="status-bar-left">
                <BackendStatus />
                <ConnectionStatus />
            </div>
            <div className="status-bar-center" />
            <div className="status-bar-right">
                <ConfigStatus />
                <UpdateStatus />
                {version && (
                    <span
                        className="status-version clickable"
                        onClick={handleNewWindow}
                        title="Open New AgentMux Window"
                    >
                        v{version}
                        {windowCount > 1 && <span className="instance-num"> ({instanceNum})</span>}
                    </span>
                )}
            </div>
        </div>
    );
});

StatusBar.displayName = "StatusBar";

export { StatusBar };
