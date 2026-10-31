// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Small pure helpers of the Files view, kept out of files-view.tsx (at its size cap). */

import type { FsGitStatus } from "@/types/rpc/FsGitStatus";

/** The branch and change count shown in the status bar. */
export function gitSummary(g: FsGitStatus): string {
    const parts: string[] = [];
    let head = g.branch ?? "detached";
    if (g.ahead) head += ` ↑${g.ahead}`;
    if (g.behind) head += ` ↓${g.behind}`;
    parts.push(head);
    if (g.changes > 0) parts.push(`${g.changes} change${g.changes === 1 ? "" : "s"}`);
    return parts.join(" · ");
}

/** The only menu actions of a pane confined to an agent's workspace folder: it lists, navigates and opens files in the Editor. */
export const CONFINED_MENU = /^(Open$|Open in Editor|Copy folder path|Show hidden files|Hide hidden files|Refresh)/;
