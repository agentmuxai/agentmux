// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { connect } from "/agentmux/widget-sdk/v1.js";

function App({ am }) {
    const [clicks, setClicks] = useState(Number(am.info.meta.clicks ?? 0));

    // Keep the pane title in step, and follow meta changed from outside.
    useEffect(() => {
        am.ui.setTitle(`Hello (${clicks})`);
    }, [clicks]);
    useEffect(() => am.on("meta", ({ meta }) => setClicks(Number(meta.clicks ?? 0))), []);

    const click = async () => {
        const next = clicks + 1;
        setClicks(next);
        await am.meta.set({ clicks: next });
    };

    return (
        <>
            <h1>Hello from React</h1>
            <p className="muted">
                Clicked {clicks} time{clicks === 1 ? "" : "s"}.
            </p>
            <button className="primary" onClick={click}>
                Click me
            </button>
        </>
    );
}

const am = await connect();
createRoot(document.getElementById("root")).render(
    <StrictMode>
        <App am={am} />
    </StrictMode>
);
