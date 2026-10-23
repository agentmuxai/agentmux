// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ☰ → Workspaces (SPEC_WORKSPACES_2026_10_23.md §3.2, §6): a switch asks
// first only when an agent is working, then closes the windows srv closed and
// opens the ones waiting; New asks for a name; Delete confirms; Export and
// Import go through the host's dialogs.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UserWorkspaceInfo, WindowAgent, WorkspaceSwitched } from "@/app/store/services";

const h = vi.hoisted(() => ({
    calls: [] as string[],
    agents: [] as WindowAgent[],
    settings: {} as Record<string, unknown>,
    modals: [] as { component: unknown; props: any }[],
    list: [] as UserWorkspaceInfo[],
    switched: null as WorkspaceSwitched | null,
    fail: null as string | null,
    dialogPath: null as string | null,
    pushNotification: vi.fn(),
}));

const { svc } = vi.hoisted(() => ({
    svc:
        (name: string, result: () => unknown) =>
        async (...args: unknown[]) => {
            h.calls.push(`${name}:${JSON.stringify(args)}`);
            if (h.fail === name) throw new Error(`${name} failed`);
            return result();
        },
}));

vi.mock("@/app/store/services", () => ({
    WorkspaceService: {
        UserWorkspaceAgents: svc("agents", () => h.agents),
        SwitchUserWorkspace: svc("switch", () => h.switched),
        CreateUserWorkspace: svc("create", () => h.switched),
        DeleteUserWorkspace: svc("delete", () => undefined),
        ExportUserWorkspace: svc("export", () => ({ path: h.dialogPath, warnings: [] })),
        PreviewImport: svc("preview", () => ({ name: "Ops", trusted: false, windows: 2, closed: 1, tabs: [{ name: "t", panes: [] }], commands: [], notes: [] })),
        ImportUserWorkspace: svc("import", () => ({ ...h.switched, notes: [] })),
    },
}));
vi.mock("@/app/store/user-workspace", () => ({
    workspaceName: () => "Default Workspace",
    refreshWorkspaceList: async () => {
        h.calls.push("list");
        return h.list;
    },
    nextWorkspaceName: (names: string[]) => `next after ${names.join(",")}`,
    renameWorkspace: async (name: string) => {
        h.calls.push(`rename:${name}`);
    },
}));
vi.mock("@/app/init/restored-windows", () => ({
    reopenRestoredWindows: async () => {
        h.calls.push("reopen-beside");
    },
}));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "win-1" }));
vi.mock("@/store/config-signals", () => ({ settingsAtom: () => h.settings }));
vi.mock("@/store/global", () => ({
    atoms: { client: () => null, muxWindow: () => ({ oid: "win-1" }) },
    openWindowEntriesAtom: () => [],
    getApi: () => ({
        closeWindow: async (label?: string) => {
            h.calls.push(`close:${label}`);
        },
        showExportWorkspaceDialog: async (name: string) => {
            h.calls.push(`export-dialog:${name}`);
            return h.dialogPath;
        },
        showImportWorkspaceDialog: async () => {
            h.calls.push("import-dialog");
            return h.dialogPath;
        },
    }),
    pushNotification: h.pushNotification,
}));
vi.mock("@/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: async (_c: unknown, patch: Record<string, unknown>) => {
            h.calls.push(`config:${JSON.stringify(patch)}`);
        },
    },
}));
vi.mock("@/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/modalmodel", () => ({
    openModal: (component: unknown, props: any) => {
        h.modals.push({ component, props });
        return { close: () => {} };
    },
    modalsModel: { isModalOpen: () => false },
}));
vi.mock("./window-close-confirm", () => ({ WindowCloseConfirm: "WindowCloseConfirm", RemoveWindowConfirm: "RemoveWindowConfirm" }));
vi.mock("./workspace-modals", () => ({
    WorkspaceNameModal: "WorkspaceNameModal",
    WorkspacePickerModal: "WorkspacePickerModal",
    DeleteWorkspaceConfirm: "DeleteWorkspaceConfirm",
    ImportPreviewModal: "ImportPreviewModal",
}));

import {
    confirmLeaving,
    deleteWorkspacePrompt,
    exportWorkspace,
    importWorkspace,
    newWorkspace,
    openWorkspacePicker,
    renameWorkspacePrompt,
    switchWarningText,
    switchWorkspace,
} from "./workspaces";

const agent = (name: string, working: boolean): WindowAgent => ({ block_id: `b-${name}`, name, working });
const ws = (id: string, name: string, current = false): UserWorkspaceInfo => ({ id, name, current, opened_at: 1, windows: 1, closed: 0 });
const lastModal = () => h.modals.at(-1)!;

beforeEach(() => {
    h.calls = [];
    h.agents = [];
    h.settings = {};
    h.modals = [];
    h.list = [ws("a", "Default Workspace", true), ws("b", "Reviews")];
    h.switched = { id: "b", name: "Reviews", closed: [{ window_id: "w2", label: "window-2" }], beside: 1 };
    h.fail = null;
    h.dialogPath = null;
    h.pushNotification.mockReset();
});

describe("the warning before a switch", () => {
    it("names who is working and says the workspace keeps its windows", () => {
        const t = switchWarningText([agent("Korp", true), agent("Loap", true), agent("Idle", false)], "Default Workspace");
        expect(t.title).toBe("Korp and Loap are working. Close Default Workspace?");
        expect(t.description).toBe(
            "This stops 3 agents. Default Workspace keeps its windows, and they open again, agents resuming, when you go back to it."
        );
        expect(switchWarningText([agent("Korp", true)], "W").title).toBe("Korp is working. Close W?");
    });

    it("isn't shown when no agent is working", async () => {
        h.agents = [agent("Idle", false)];
        expect(await confirmLeaving()).toBe(true);
        expect(h.modals).toEqual([]);
    });

    it("is shown when one is, and its answer decides", async () => {
        h.agents = [agent("Korp", true)];
        const yes = confirmLeaving();
        await vi.waitFor(() => expect(h.modals).toHaveLength(1));
        expect(lastModal().component).toBe("WindowCloseConfirm");
        expect(lastModal().props.title).toBe("Korp is working. Close Default Workspace?");
        expect(lastModal().props.confirmLabel).toBe("Close and switch");
        lastModal().props.onConfirm(true);
        expect(await yes).toBe(true);
        expect(h.calls).toContain('config:{"window:confirmcloseworking":false}');

        const no = confirmLeaving();
        await vi.waitFor(() => expect(h.modals).toHaveLength(2));
        lastModal().props.onCancel();
        expect(await no).toBe(false);
    });

    it("follows the window:confirmcloseworking setting", async () => {
        h.agents = [agent("Korp", true)];
        h.settings = { "window:confirmcloseworking": false };
        expect(await confirmLeaving()).toBe(true);
        expect(h.modals).toEqual([]);
    });
});

describe("switching", () => {
    it("asks srv, closes the windows it closed and opens the ones waiting beside", async () => {
        await switchWorkspace(ws("b", "Reviews"));
        expect(h.calls).toEqual(["agents:[]", 'switch:["b","win-1"]', "close:window-2", "reopen-beside", "list"]);
    });

    it("does nothing for the open workspace", async () => {
        await switchWorkspace(ws("a", "Default Workspace", true));
        expect(h.calls).toEqual([]);
    });

    it("does nothing when the user keeps the workspace open", async () => {
        h.agents = [agent("Korp", true)];
        const done = switchWorkspace(ws("b", "Reviews"));
        await vi.waitFor(() => expect(h.modals).toHaveLength(1));
        lastModal().props.onCancel();
        await done;
        expect(h.calls.some((c) => c.startsWith("switch:"))).toBe(false);
    });

    it("says so when srv can't", async () => {
        h.fail = "switch";
        await switchWorkspace(ws("b", "Reviews"));
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ type: "error", title: "Couldn't open Reviews", message: "switch failed" }));
        expect(h.calls.some((c) => c.startsWith("close:"))).toBe(false);
    });

    it("the palette's Open Workspace… lists every workspace and opens the one chosen", async () => {
        await openWorkspacePicker();
        expect(lastModal().component).toBe("WorkspacePickerModal");
        expect(lastModal().props.workspaces).toEqual(h.list);
        lastModal().props.onPick(h.list[1]);
        await vi.waitFor(() => expect(h.calls).toContain('switch:["b","win-1"]'));
    });
});

describe("New Workspace…", () => {
    it("asks for a name, prefilled with the next free one, then creates and switches", async () => {
        h.switched = { id: "n", name: "Ops", closed: [], beside: 0 };
        await newWorkspace();
        const m = lastModal();
        expect(m.component).toBe("WorkspaceNameModal");
        expect(m.props.initial).toBe("next after Default Workspace,Reviews");
        expect(m.props.confirmLabel).toBe("Create");
        await m.props.onSubmit("Ops");
        expect(h.calls).toContain('create:["Ops","win-1"]');
        expect(h.calls).not.toContain("reopen-beside");
    });
});

describe("Rename workspace…", () => {
    it("asks for the new name, prefilled with the current one", async () => {
        renameWorkspacePrompt();
        expect(lastModal().props.initial).toBe("Default Workspace");
        await lastModal().props.onSubmit("Reviews");
        expect(h.calls).toContain("rename:Reviews");
    });
});

describe("Delete workspace…", () => {
    it("lists the other workspaces, confirms, then deletes", async () => {
        await deleteWorkspacePrompt();
        expect(lastModal().component).toBe("WorkspacePickerModal");
        expect(lastModal().props.workspaces.map((w: UserWorkspaceInfo) => w.id)).toEqual(["b"]);
        lastModal().props.onPick(h.list[1]);
        expect(lastModal().component).toBe("DeleteWorkspaceConfirm");
        expect(lastModal().props.workspace.id).toBe("b");
        await lastModal().props.onConfirm();
        expect(h.calls).toContain('delete:["b"]');
    });

    it("says so when only the open workspace exists", async () => {
        h.list = [ws("a", "Default Workspace", true)];
        await deleteWorkspacePrompt();
        expect(h.modals).toEqual([]);
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ title: "No other workspace to delete" }));
    });
});

describe("Export… and Import…", () => {
    it("Export… writes the file the dialog picked, and nothing when cancelled", async () => {
        await exportWorkspace();
        expect(h.calls).toEqual(["export-dialog:Default Workspace"]);
        h.dialogPath = "C:/w/Default Workspace.agentmux-workspace.json";
        await exportWorkspace();
        expect(h.calls).toContain(`export:${JSON.stringify([h.dialogPath])}`);
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ type: "info", message: h.dialogPath }));
    });

    it("Import… previews the file, then imports it as a new workspace and switches", async () => {
        h.dialogPath = "C:/w/Ops.agentmux-workspace.json";
        h.switched = { id: "n", name: "Ops", closed: [], beside: 1 };
        await importWorkspace();
        expect(lastModal().component).toBe("ImportPreviewModal");
        expect(lastModal().props.current).toBe("Default Workspace");
        await lastModal().props.onImport(false);
        expect(h.calls).toContain(`import:${JSON.stringify([h.dialogPath, false, "win-1"])}`);
        expect(h.calls).toContain("reopen-beside");
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ title: "Imported Ops", message: "2 windows, 1 tab." }));
    });

    it("Import… reports a file it can't read", async () => {
        h.dialogPath = "C:/w/bad.agentmux-workspace.json";
        h.fail = "preview";
        await importWorkspace();
        expect(h.modals).toEqual([]);
        expect(h.pushNotification).toHaveBeenCalledWith(expect.objectContaining({ type: "error", title: "Couldn't read the file" }));
    });
});
