// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getApi, windowCountAtom, windowInstanceNumAtom } from "@/store/global";
import { useAtomValue } from "jotai";
import { memo } from "react";
import "./window-controls.scss";

const WindowControls = memo(() => {
    const instanceNum = useAtomValue(windowInstanceNumAtom);
    const windowCount = useAtomValue(windowCountAtom);

    const handleNewWindow = async () => {
        try {
            const newWindowLabel = await getApi().openNewWindow();
            console.log("[WindowControls] Opened new window:", newWindowLabel);
        } catch (error) {
            console.error("[WindowControls] Failed to open new window:", error);
        }
    };

    const version = getApi().getAboutModalDetails()?.version ?? "?";

    return (
        <div className="window-controls" data-tauri-drag-region="false" data-testid="window-controls">
            <button
                className="window-control-btn new-window-btn"
                onClick={handleNewWindow}
                title="Open New Window"
                data-testid="new-window-btn"
            >
                <i className="fa fa-window-restore" />
                <span>
                    agentmux v{version}
                    {windowCount > 1 && <span className="instance-num"> ({instanceNum})</span>}
                </span>
            </button>
        </div>
    );
});

export { WindowControls };
