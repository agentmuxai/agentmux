// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/store/global";
import { memo } from "react";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { ConnectionStatus } from "./ConnectionStatus";
import { UpdateStatus } from "./UpdateStatus";
import "./StatusBar.scss";

const StatusBar = memo(() => {
    const version = getApi().getAboutModalDetails()?.version ?? "";

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
                {version && <span className="status-version">v{version}</span>}
            </div>
        </div>
    );
});

StatusBar.displayName = "StatusBar";

export { StatusBar };
