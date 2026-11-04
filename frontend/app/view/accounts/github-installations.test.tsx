// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A brokered GitHub account's installations: what the rows say, and the install trip
// (Connect with GitHub, SPEC_CONNECT_WITH_GITHUB §4.2).

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { GithubInstallationsCommand: rpc.call } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { linkedLine, NO_INSTALLATIONS_HINT, problemLine, reposLine, SYNC_EVERY_MS, SYNC_GIVE_UP_MS, waitingGivesUp } from "./github-installations";
import { GithubInstallationsSection } from "./GithubInstallationsSection";

const row = (id: string, login: string, over: Record<string, unknown> = {}) => ({
    installationId: id,
    accountLogin: login,
    repositorySelection: "selected",
    status: "active",
    visibleCount: 3,
    verifiedAtMs: 1,
    ...over,
});
const answer = (installations: unknown[], over: Record<string, unknown> = {}) => ({ signedIn: true, installations, linked: [], ...over });

describe("what a row says", () => {
    it("names the repositories and the state in words", () => {
        expect(reposLine({ repositorySelection: "all", visibleCount: 0 })).toBe("All repositories");
        expect(reposLine({ repositorySelection: "selected", visibleCount: 1 })).toBe("1 repository you can see");
        expect(reposLine({ repositorySelection: "selected", visibleCount: 4 })).toBe("4 repositories you can see");
        expect(problemLine({ status: "active" })).toBeNull();
        expect(problemLine({ status: "suspended" })).toBe("Suspended on GitHub");
        expect(problemLine({ status: "deleted" })).toBe("Removed from GitHub");
        expect(problemLine({ status: "revoked" })).toContain("Access lost");
    });

    it("says what was linked, by name when the list has it", () => {
        const rows = [row("1", "octo"), row("2", "acme")];
        expect(linkedLine([], rows)).toBeNull();
        expect(linkedLine(["2"], rows)).toBe("Linked @acme.");
        expect(linkedLine(["1", "2"], rows)).toBe("Linked @octo, @acme.");
        expect(linkedLine(["9"], rows)).toBe("Linked 1 installation.");
        expect(linkedLine(["8", "9"], rows)).toBe("Linked 2 installations.");
    });

    it("stops waiting after ten minutes", () => {
        expect(waitingGivesUp(0, SYNC_GIVE_UP_MS - 1)).toBe(false);
        expect(waitingGivesUp(0, SYNC_GIVE_UP_MS)).toBe(true);
    });
});

describe("the installations section", () => {
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        rpc.call.mockReset();
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("lists the account's installations and says so when there are none", async () => {
        rpc.call.mockResolvedValueOnce(answer([row("1", "octo"), row("2", "acme", { repositorySelection: "all", status: "suspended" })]));
        render(() => <GithubInstallationsSection />);
        await screen.findByText("@octo");
        expect(screen.getByText("@acme")).toBeTruthy();
        expect(screen.getByText(/All repositories/)).toBeTruthy();
        expect(screen.getByText(/Suspended on GitHub/)).toBeTruthy();
        expect(rpc.call).toHaveBeenCalledWith({}, { action: "list" });
        cleanup();
        rpc.call.mockResolvedValueOnce(answer([]));
        render(() => <GithubInstallationsSection />);
        await screen.findByText(NO_INSTALLATIONS_HINT);
    });

    it("installs: opens GitHub, then asks for the new installation until it is linked", async () => {
        rpc.call.mockResolvedValueOnce(answer([]));
        render(() => <GithubInstallationsSection />);
        await screen.findByText(NO_INSTALLATIONS_HINT);

        rpc.call.mockResolvedValueOnce(answer([], { installUrl: "https://github.com/apps/muxagent/installations/new" }));
        fireEvent.click(screen.getByText("Install on repositories…"));
        await screen.findByText(/Waiting for you to finish on GitHub/);
        expect(rpc.call).toHaveBeenLastCalledWith({}, { action: "start" });

        // Nothing new yet: it keeps waiting. Then the installation is there and linked.
        rpc.call.mockResolvedValueOnce(answer([]));
        await vi.advanceTimersByTimeAsync(SYNC_EVERY_MS);
        expect(rpc.call).toHaveBeenLastCalledWith({}, { action: "sync" });
        expect(screen.getByText(/Waiting for you to finish on GitHub/)).toBeTruthy();
        rpc.call.mockResolvedValueOnce(answer([row("5", "acme")], { linked: ["5"] }));
        await vi.advanceTimersByTimeAsync(SYNC_EVERY_MS);
        await screen.findByText("Linked @acme.");
        expect(screen.queryByText(/Waiting for you to finish on GitHub/)).toBeNull();

        // It has stopped asking.
        const calls = rpc.call.mock.calls.length;
        await vi.advanceTimersByTimeAsync(SYNC_EVERY_MS * 3);
        expect(rpc.call.mock.calls.length).toBe(calls);
    });

    it("gives up after ten minutes without an installation", async () => {
        rpc.call.mockResolvedValue(answer([]));
        render(() => <GithubInstallationsSection />);
        await screen.findByText(NO_INSTALLATIONS_HINT);
        fireEvent.click(screen.getByText("Install on repositories…"));
        await screen.findByText(/Waiting for you to finish on GitHub/);
        await vi.advanceTimersByTimeAsync(SYNC_GIVE_UP_MS + SYNC_EVERY_MS);
        await waitFor(() => expect(screen.queryByText(/Waiting for you to finish on GitHub/)).toBeNull());
    });

    it("unlinks one installation and shows the list after", async () => {
        rpc.call.mockResolvedValueOnce(answer([row("1", "octo"), row("2", "acme")]));
        render(() => <GithubInstallationsSection />);
        await screen.findByText("@acme");
        rpc.call.mockResolvedValueOnce(answer([row("1", "octo")]));
        fireEvent.click(screen.getAllByText("Unlink")[1]);
        await waitFor(() => expect(screen.queryByText("@acme")).toBeNull());
        expect(rpc.call).toHaveBeenLastCalledWith({}, { action: "unlink", installationId: "2" });
    });

    it("shows what went wrong, and asks for the Cloud sign-in when there is none", async () => {
        rpc.call.mockResolvedValueOnce(answer([row("1", "octo")], { error: "that install was started too long ago or already used: start it again" }));
        render(() => <GithubInstallationsSection />);
        await screen.findByText(/start it again/);
        cleanup();
        rpc.call.mockResolvedValueOnce({ signedIn: false, installations: [], linked: [], error: "sign in to AgentMux Cloud first" });
        render(() => <GithubInstallationsSection />);
        await screen.findByText(/Sign in to AgentMux Cloud to see the installations/);
    });
});
