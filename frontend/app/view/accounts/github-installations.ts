// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The installations of the AgentMux GitHub App a GitHub account is linked to, in its details
// (Connect with GitHub, phase G2): what each row says, and the calls the section makes.
// srv does the work (`github.installations`, crates/srv/src/identity/github_installs.rs).

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { GithubInstallation } from "@/types/rpc/GithubInstallation";
import type { GithubInstallations } from "@/types/rpc/GithubInstallations";
import type { GithubInstallationsAction } from "@/types/rpc/GithubInstallationsAction";

export type { GithubInstallation, GithubInstallations };

/** How often the section asks GitHub for a new installation while the person is on its install page. */
export const SYNC_EVERY_MS = 4000;
/** How long it keeps asking. */
export const SYNC_GIVE_UP_MS = 10 * 60 * 1000;

export function callInstallations(action: GithubInstallationsAction, installationId?: string): Promise<GithubInstallations> {
    return RpcApi.GithubInstallationsCommand(TabRpcClient, { action, ...(installationId ? { installationId } : {}) });
}

/** Whether waiting for an install should stop. */
export function waitingGivesUp(startedMs: number, nowMs: number): boolean {
    return nowMs - startedMs >= SYNC_GIVE_UP_MS;
}

/** What the repositories of a row are: all of the account's, or the chosen ones this account can see. */
export function reposLine(i: Pick<GithubInstallation, "repositorySelection" | "visibleCount">): string {
    if (i.repositorySelection === "all") return "All repositories";
    const n = i.visibleCount;
    return n === 1 ? "1 repository you can see" : `${n} repositories you can see`;
}

/** The state of a row in words; null when it is simply working. */
export function problemLine(i: Pick<GithubInstallation, "status">): string | null {
    switch (i.status) {
        case "active":
            return null;
        case "suspended":
            return "Suspended on GitHub";
        case "deleted":
            return "Removed from GitHub";
        case "revoked":
            return "Access lost: check GitHub again";
        default:
            return i.status;
    }
}

/** What the section says after a call linked some installations; null if it linked none. */
export function linkedLine(linked: string[], rows: Pick<GithubInstallation, "installationId" | "accountLogin">[]): string | null {
    if (linked.length === 0) return null;
    const names = linked.map((id) => rows.find((r) => r.installationId === id)?.accountLogin).filter((n): n is string => !!n);
    if (names.length === 0) return linked.length === 1 ? "Linked 1 installation." : `Linked ${linked.length} installations.`;
    return `Linked ${names.map((n) => `@${n}`).join(", ")}.`;
}

/** The hint under an empty list. */
export const NO_INSTALLATIONS_HINT =
    "The AgentMux GitHub App isn't installed on any repositories yet. Install it on the ones your agents should work on.";
