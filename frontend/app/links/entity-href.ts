// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The address a link made from text shows and copies, and the web page one stands for (spec §8). */

import { pullUrl } from "./entities/kinds/pull";
import type { Destination } from "./entities/types";

/** The address a link shows and copies: the page itself, or the app's own address for a place in it. */
export function hrefOf(dest: Destination): string {
    switch (dest.kind) {
        case "url":
            return dest.url;
        case "pull":
            return pullUrl(dest);
        case "task":
            return `agentmux://task/${dest.number}`;
        case "agent":
            return `agentmux://agent/${encodeURIComponent(dest.id)}`;
        case "file":
            return `agentmux://file?path=${encodeURIComponent(dest.path)}`;
    }
}

/** The web page a destination is, if it is one: it opens as every web link does. */
export function webUrlOf(dest: Destination): string | null {
    return dest.kind === "url" ? dest.url : dest.kind === "pull" ? pullUrl(dest) : null;
}
