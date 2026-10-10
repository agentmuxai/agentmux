// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { connect, useVisibility } from "/agentmux/widget-sdk/v1.js";

// The handshake. It also applies AgentMux's theme as CSS variables
// (am-widget.css uses them) and keeps them current.
const am = await connect();

const countEl = document.getElementById("count");
const whereEl = document.getElementById("where");

// The click count lives in this pane's meta: it survives restarts and moves
// with the pane, and each pane of this widget has its own.
let clicks = Number(am.info.meta.clicks ?? 0);

function render() {
    countEl.textContent = `Clicked ${clicks} time${clicks === 1 ? "" : "s"}.`;
    am.ui.setTitle(`Hello (${clicks})`);
}

document.getElementById("click").addEventListener("click", async () => {
    clicks += 1;
    render();
    await am.meta.set({ clicks });
});

// A header action, drawn by AgentMux; its click comes back as an event.
await am.ui.setHeaderActions([{ id: "reset", icon: "rotate-left", title: "Reset the count" }]);
am.on("action", async ({ id }) => {
    if (id !== "reset") return;
    clicks = 0;
    render();
    await am.meta.set({ clicks: null });
    await am.ui.toast("Count reset", "success");
});

// The pane's meta can change from outside (another window, an undo).
am.on("meta", ({ meta }) => {
    clicks = Number(meta.clicks ?? 0);
    render();
});

useVisibility(am, {
    onActive: () => (whereEl.textContent = `Running in AgentMux ${am.info.agentmux.version}, pane "${am.info.widget.pane}".`),
    onDormant: () => console.log("hidden: a widget with timers would pause them here"),
});

render();
