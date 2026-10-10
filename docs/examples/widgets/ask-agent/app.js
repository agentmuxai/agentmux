// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { connect, useVisibility } from "/agentmux/widget-sdk/v1.js";

const am = await connect();
const select = document.getElementById("agent");
const statusEl = document.getElementById("status");

async function refreshAgents() {
    const agents = await am.agents.list();
    const chosen = select.value || am.info.meta.agent;
    select.replaceChildren(
        ...agents.map((a) => Object.assign(document.createElement("option"), { value: a.id, textContent: `${a.name} (${a.state})`, className: `state-${a.state}` }))
    );
    if (agents.some((a) => a.id === chosen)) select.value = chosen;
    if (!agents.length) statusEl.textContent = "No agents are running on this computer.";
}

select.addEventListener("change", () => am.meta.set({ agent: select.value }));

document.getElementById("ask").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const box = document.getElementById("text");
    const text = box.value.trim();
    if (!select.value || !text) return;
    try {
        await am.agents.send(select.value, text);
        box.value = "";
        statusEl.className = "success";
        statusEl.textContent = `Sent to ${select.value}.`;
    } catch (e) {
        // Not running any more, or over the rate limit.
        statusEl.className = "error";
        statusEl.textContent = e.message;
    }
});

// Keep the working/idle states current while the pane is shown.
let timer = 0;
useVisibility(am, {
    onActive: () => {
        refreshAgents();
        timer = setInterval(refreshAgents, 15000);
    },
    onDormant: () => clearInterval(timer),
});
