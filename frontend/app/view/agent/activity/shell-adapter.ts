// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shell adapter — maps `ShellNode`s (agent-document store) onto `PinnedActivity`
 * for the dock. The dock is a pure derived view; no new state.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md (§3, D2)
 */

import type { DocumentNode, ShellNode } from "../types";
import type { PinnedActivity, ActivityStatus } from "./types";

function shellStatusToActivity(s: ShellNode["status"]): ActivityStatus {
    switch (s) {
        case "running": return "running";
        case "stopped": return "stopped";
        case "exited-err": return "error";
        case "exited-ok": return "done";
    }
}

function shellToActivity(n: ShellNode): PinnedActivity {
    return {
        id: n.id,
        kind: "shell",
        title: n.title,
        status: shellStatusToActivity(n.status),
        startedAt: n.spawnedAt,
        endedAt: n.exitedAt,
        canStop: n.status === "running",
        shell: n,
    };
}

/** Pull every shell node out of the document and map to activities. */
export function shellActivities(nodes: ReadonlyArray<DocumentNode>): PinnedActivity[] {
    const out: PinnedActivity[] = [];
    for (const n of nodes) {
        if (n.type === "shell") out.push(shellToActivity(n as ShellNode));
    }
    return out;
}
