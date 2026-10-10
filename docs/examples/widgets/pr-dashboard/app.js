// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { connect, useVisibility } from "/agentmux/widget-sdk/v1.js";

const am = await connect();
const statusEl = document.getElementById("status");
const listEl = document.getElementById("prs");
const repoInput = document.getElementById("repo");

let repo = String(am.info.meta.repo ?? "agentmuxai/agentmux");
repoInput.value = repo;

async function refresh() {
    statusEl.className = "muted";
    statusEl.textContent = `Loading ${repo}…`;
    try {
        const resp = await am.net.fetch(`https://api.github.com/repos/${repo}/pulls?state=open&per_page=30`, {
            headers: { Accept: "application/vnd.github+json" },
        });
        if (!resp.ok) throw new Error(`GitHub answered ${resp.status} ${resp.statusText}`);
        const prs = resp.json();
        listEl.replaceChildren(
            ...prs.map((pr) => {
                const li = document.createElement("li");
                const link = Object.assign(document.createElement("a"), { href: pr.html_url, textContent: `#${pr.number} ${pr.title}` });
                // Links open in a browser pane next to this one.
                link.onclick = (ev) => {
                    ev.preventDefault();
                    am.ui.openUrl(pr.html_url);
                };
                const by = Object.assign(document.createElement("div"), { className: "muted", textContent: `${pr.user.login}${pr.draft ? " · draft" : ""}` });
                li.append(link, by);
                return li;
            })
        );
        statusEl.textContent = `${prs.length} open in ${repo}, as of ${new Date().toLocaleTimeString()}.`;
        await am.ui.setTitle(`PRs: ${repo} (${prs.length})`);
        // The status bar item, while this pane is open.
        await am.ui.setStatusItem("count", { text: `${prs.length} PRs`, tooltip: `Open pull requests in ${repo}` });
    } catch (e) {
        statusEl.className = "error";
        statusEl.textContent = e.message;
    }
}

document.getElementById("pick").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const next = repoInput.value.trim();
    if (!/^[\w.-]+\/[\w.-]+$/.test(next)) return am.ui.toast("Write it as owner/repo", "warning");
    repo = next;
    await am.meta.set({ repo });
    await refresh();
});

await am.ui.setHeaderActions([{ id: "refresh", icon: "rotate-right", title: "Refresh" }]);
am.on("action", ({ id }) => id === "refresh" && refresh());
// "Refresh pull requests" in the command palette, or a click on the status item.
am.on("command", ({ id }) => id === "refresh" && refresh());

// Poll only while the pane is shown.
let timer = 0;
useVisibility(am, {
    onActive: () => {
        refresh();
        timer = setInterval(refresh, 120_000);
    },
    onDormant: () => clearInterval(timer),
});
