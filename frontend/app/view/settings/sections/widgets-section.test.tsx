// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Widgets: the package list, the install prompt and its wording,
 *  approval through the host, and uninstall
 *  (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.3, §8.4). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";

const list = vi.fn();
const install = vi.fn();
const uninstall = vi.fn();
const setEnabled = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        WidgetsListCommand: (...a: unknown[]) => list(...a),
        WidgetsInstallCommand: (...a: unknown[]) => install(...a),
        WidgetsUninstallCommand: (...a: unknown[]) => uninstall(...a),
        WidgetsSetEnabledCommand: (...a: unknown[]) => setEnabled(...a),
        WidgetsRescanCommand: (...a: unknown[]) => list(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/block/widget-loader", () => ({ widgetLoadErrors: () => ({}) }));
vi.mock("@/app/view/agent/agent-launch-env", () => ({ agentmuxHome: () => "C:/Users/me/.agentmux" }));
const decideWidget = vi.fn(async () => {});
const forgetWidgetKey = vi.fn(async () => {});
const showOpenFileDialog = vi.fn(async () => "C:/src/acme.notes/widget.json");
vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({ approvals: { decideWidget, forgetWidgetKey }, showOpenFileDialog, openNativePath: vi.fn() }),
}));

import { setWidgetPackages } from "@/app/store/widget-packages-store";
import { WidgetsSection } from "./widgets-section";

function pkg(over: Partial<WidgetPackageInfo> = {}): WidgetPackageInfo {
    return {
        id: "acme.notes",
        name: "Notes",
        version: "1.0.0",
        description: "A scratchpad",
        author: "Acme",
        homepage: null,
        icon: "note-sticky",
        default_hue: null,
        kind: "sandboxed",
        permissions: ["storage", "net:https://api.github.com", "agents:send"],
        granted: [],
        state: "needs_approval",
        error: null,
        hash: "abc123",
        panes: [],
        commands: [],
        status_items: [],
        signature: { state: "unsigned", publisher: "acme", fingerprint: null, pinned: null },
        files_url: null,
        implied: false,
        folder: "C:/Users/me/.agentmux/widgets/acme.notes",
        ...over,
    };
}

/** The store's list, and what srv answers when the section fetches it. */
function given(packages: WidgetPackageInfo[]): void {
    setWidgetPackages(packages);
    list.mockResolvedValue({ packages });
}

describe("WidgetsSection", () => {
    beforeEach(() => {
        for (const f of [list, install, uninstall, setEnabled, decideWidget, forgetWidgetKey, showOpenFileDialog]) f.mockClear();
        list.mockResolvedValue({ packages: [] });
    });
    afterEach(() => cleanup());

    it("says what each permission means before the user approves, and approves through the host", async () => {
        given([pkg()]);
        render(() => <WidgetsSection />);
        const row = (await screen.findByText("Notes")).closest(".widget-row") as HTMLElement;
        expect(within(row).getByText("Waiting for your approval")).toBeInTheDocument();
        fireEvent.click(within(row).getByText("Approve"));
        const prompt = within(row).getByRole("dialog");
        expect(within(prompt).getByText("Keep its own data on this computer")).toBeInTheDocument();
        expect(within(prompt).getByText("Connect to https://api.github.com")).toBeInTheDocument();
        expect(within(prompt).getByText(/Send messages to your agents/)).toBeInTheDocument();
        fireEvent.click(within(prompt).getByText("Install"));
        await waitFor(() => expect(decideWidget).toHaveBeenCalledWith("acme.notes", "abc123", true, ""));
    });

    it("warns, in so many words, that a trusted widget has full access", async () => {
        given([pkg({ kind: "trusted", permissions: [] })]);
        render(() => <WidgetsSection />);
        const row = (await screen.findByText("Notes")).closest(".widget-row") as HTMLElement;
        fireEvent.click(within(row).getByText("Approve"));
        expect(within(row).getByText(/full access to everything AgentMux can do/)).toBeInTheDocument();
    });

    it("installs a picked package and opens its prompt", async () => {
        given([]);
        install.mockResolvedValue({ id: "acme.notes", packages: [pkg()] });
        render(() => <WidgetsSection />);
        fireEvent.click(screen.getByText("Install…"));
        await waitFor(() => expect(install).toHaveBeenCalledWith({}, { path: "C:/src/acme.notes/widget.json", replace: false }));
        expect(await screen.findByRole("dialog")).toBeInTheDocument();
    });

    it("asks before replacing an installed package", async () => {
        given([pkg({ state: "approved" })]);
        install.mockRejectedValueOnce(new Error("acme.notes is already installed"));
        install.mockResolvedValueOnce({ id: "acme.notes", packages: [pkg({ state: "needs_approval" })] });
        render(() => <WidgetsSection />);
        fireEvent.click(screen.getByText("Install…"));
        fireEvent.click(await screen.findByText("Replace"));
        await waitFor(() => expect(install).toHaveBeenLastCalledWith({}, { path: "C:/src/acme.notes/widget.json", replace: true }));
    });

    it("uninstalls only after a second click", async () => {
        given([pkg({ state: "approved" })]);
        uninstall.mockResolvedValue({ packages: [] });
        render(() => <WidgetsSection />);
        fireEvent.click(await screen.findByText("Uninstall"));
        expect(uninstall).not.toHaveBeenCalled();
        fireEvent.click(screen.getByText("Delete"));
        await waitFor(() => expect(uninstall).toHaveBeenCalledWith({}, { id: "acme.notes" }));
    });

    it("can't uninstall a widget that comes from widgets.json", async () => {
        given([pkg({ implied: true, kind: "trusted", state: "approved" })]);
        render(() => <WidgetsSection />);
        await screen.findByText("Notes");
        expect(screen.queryByText("Uninstall")).not.toBeInTheDocument();
    });

    it("says who signed a widget, and warns when it isn't the publisher's usual key", async () => {
        const fp = "K7Q2-MZ4D-PX3A-9TWE";
        given([
            pkg({ signature: { state: "signed_new", publisher: "acme", fingerprint: fp, pinned: null } }),
            pkg({ id: "acme.board", name: "Board", hash: "def", signature: { state: "key_changed", publisher: "acme", fingerprint: "AAAA-BBBB-CCCC-DDDD", pinned: fp } }),
        ]);
        render(() => <WidgetsSection />);
        const notes = (await screen.findByText("Notes")).closest(".widget-row") as HTMLElement;
        expect(within(notes).getByText(`Signed by acme · ${fp}`)).toBeInTheDocument();
        fireEvent.click(within(notes).getByText("Approve"));
        expect(within(notes).getByText(/This is the first acme widget here/)).toBeInTheDocument();
        expect(within(notes).getByText("Install")).toBeInTheDocument();

        const board = screen.getByText("Board").closest(".widget-row") as HTMLElement;
        fireEvent.click(within(board).getByText("Approve"));
        const prompt = within(board).getByRole("dialog");
        expect(within(prompt).getByText(/Not signed by the key that signed your other acme widgets/)).toBeInTheDocument();
        fireEvent.click(within(prompt).getByText("Install anyway"));
        await waitFor(() => expect(decideWidget).toHaveBeenCalledWith("acme.board", "def", true, "AAAA-BBBB-CCCC-DDDD"));
    });

    it("lists pinned publisher keys and forgets one through the host", async () => {
        given([pkg()]);
        list.mockResolvedValue({ packages: [pkg()], publishers: [{ publisher: "acme", fingerprint: "K7Q2-MZ4D-PX3A-9TWE" }] });
        const { refreshWidgetPackages } = await import("@/app/store/widget-packages-store");
        await refreshWidgetPackages();
        render(() => <WidgetsSection />);
        const row = (await screen.findByText("K7Q2-MZ4D-PX3A-9TWE")).closest(".widget-publisher-row") as HTMLElement;
        fireEvent.click(within(row).getByText("Forget key"));
        await waitFor(() => expect(forgetWidgetKey).toHaveBeenCalledWith("acme"));
    });
});
