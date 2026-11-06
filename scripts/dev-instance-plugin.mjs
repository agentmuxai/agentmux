// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Lets `task dev` tell its own Vite from another run's: the dev server answers
// /__agentmux_dev_instance with the token the run gave it (AGENTMUX_DEV_INSTANCE),
// and scripts/dev-vite-wait.sh refuses a server that answers with another token or
// none. Without it a Vite that loses the --strictPort bind exits quietly, a plain
// "does anything answer" check passes against the winner, and the window loads
// someone else's frontend.
// docs/retro/RETRO_DEV_VITE_PORT_RACE_COUPLED_INSTANCES_2026_11_06.md

export const DEV_INSTANCE_PATH = "/__agentmux_dev_instance";

/** @returns {import("vite").Plugin} */
export function devInstance() {
    const token = process.env.AGENTMUX_DEV_INSTANCE;
    return {
        name: "agentmux-dev-instance",
        apply: "serve",
        configureServer(server) {
            if (!token) return;
            const body = JSON.stringify({
                token,
                owner: process.env.AGENTMUX_DEV_OWNER ?? null,
                commit: process.env.AGENTMUX_DEV_COMMIT ?? null,
                root: server.config.root,
            });
            server.middlewares.use(DEV_INSTANCE_PATH, (_req, res) => {
                res.setHeader("content-type", "application/json");
                res.setHeader("cache-control", "no-store");
                res.end(body);
            });
        },
    };
}
