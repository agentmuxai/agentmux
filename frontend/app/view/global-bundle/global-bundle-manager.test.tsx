// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleManager — Armory → Memory → Global as TILES FIRST, expanding to
// a full view (docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3),
// with the editor pinned to the bottom, a base-carrying save and dirty-draft
// protection (§2.4).
//
// The read-only "Claude Code provider config" CLAUDE.md (the file in the
// isolated config dir a spawned agent actually launches with — NOT part of
// AgentMux's own Global Memory composition, codex P1 PR #2794;
// SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §4, §7) is a lock-badged
// tile now; its path and content show in its full view. The sibling
// ~/.claude host-CLI-config display stays gone
// (SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md).

import { createHash } from "node:crypto";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { Bundle } from "@/app/store/rpc-api";

const CLAUDE_CONFIG_PATH = "/home/user/.agentmux/shared/providers/claude/CLAUDE.md";

const listMemoriesMock = vi.fn();
const getClaudeGlobalConfigMock = vi.fn();
const upsertMock = vi.fn();
const upsertSystemMock = vi.fn();
const reorderMock = vi.fn();
const historyMock = vi.fn();
const diffMock = vi.fn();
const revertMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListBundlesCommand: (...args: unknown[]) => listMemoriesMock(...args),
        GetClaudeGlobalConfigCommand: (...args: unknown[]) => getClaudeGlobalConfigMock(...args),
        UpsertBundleCommand: (...args: unknown[]) => upsertMock(...args),
        UpsertSystemBundleCommand: (...args: unknown[]) => upsertSystemMock(...args),
        ReorderGlobalBundlesCommand: (...args: unknown[]) => reorderMock(...args),
        GlobalMemoryHistoryCommand: (...args: unknown[]) => historyMock(...args),
        GlobalMemoryDiffCommand: (...args: unknown[]) => diffMock(...args),
        GlobalMemoryRevertCommand: (...args: unknown[]) => revertMock(...args),
    },
}));

const mpsHub = vi.hoisted(() => ({ handlers: new Map<string, () => void>() }));
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: vi.fn((sub: { eventType: string; handler: () => void }) => {
        mpsHub.handlers.set(sub.eventType, sub.handler);
        return () => mpsHub.handlers.delete(sub.eventType);
    }),
}));

import { GlobalBundleManager } from "./global-bundle-manager";

function bundle(id: string, name: string, instructions: string, over: Partial<Bundle> = {}): Bundle {
    return {
        id,
        name,
        description: "",
        is_blank: false,
        is_global: true,
        provider: "",
        model: "",
        instructions,
        instructions_by_provider: "{}",
        context_files: "[]",
        mcp_servers: "[]",
        skills: "[]",
        sort_order: 0,
        is_system: false,
        created_at: 1,
        updated_at: Date.now() - 2 * 3_600_000,
        ...over,
    };
}

/** The server's content_hash for a Global Memory entry. */
const serverHash = (name: string, instructions: string) =>
    createHash("sha256").update(`${name}\0${instructions}`).digest("hex");

const tileTitles = () =>
    Array.from(document.querySelectorAll('[data-testid="global-memory-tile"] .memory-file-card-title')).map(
        (el) => el.textContent
    );

async function openTile(title: string): Promise<void> {
    const tile = await waitFor(() => {
        const el = Array.from(document.querySelectorAll<HTMLElement>('[data-testid="global-memory-tile"]')).find(
            (t) => t.querySelector(".memory-file-card-title")?.textContent === title
        );
        if (!el) throw new Error(`no tile ${title}`);
        return el;
    });
    fireEvent.click(tile);
}

const textarea = () => document.querySelector<HTMLTextAreaElement>(".memory-content-textarea");

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

beforeEach(() => {
    for (const m of [listMemoriesMock, getClaudeGlobalConfigMock, upsertMock, upsertSystemMock, reorderMock, historyMock, diffMock, revertMock]) {
        m.mockReset();
    }
    mpsHub.handlers.clear();
    localStorage.clear();
    listMemoriesMock.mockResolvedValue([]);
    getClaudeGlobalConfigMock.mockResolvedValue({ path: CLAUDE_CONFIG_PATH, content: null, exists: false });
    historyMock.mockResolvedValue({ versions: [] });
    upsertMock.mockImplementation((_c: unknown, b: Bundle) => Promise.resolve({ ...b, id: b.id || "new-id" }));
    reorderMock.mockResolvedValue({ updated: 1 });
});

describe("GlobalBundleManager — tiles first", () => {
    test("one tile per entry (system first, badged), then CLAUDE.md, Combined preview, and + Add memory last", async () => {
        listMemoriesMock.mockResolvedValue([
            bundle("g-b", "Beta", "b", { sort_order: 1 }),
            bundle("g-a", "Alpha", "hello", { sort_order: 0 }),
            bundle("sys-1", "Policy", "p", { is_system: true }),
            bundle("private", "Not global", "x", { is_global: false }),
        ]);
        getClaudeGlobalConfigMock.mockResolvedValue({ path: CLAUDE_CONFIG_PATH, content: "# rules\n", exists: true });
        render(() => <GlobalBundleManager />);

        await waitFor(() =>
            expect(tileTitles()).toEqual(["Policy", "Alpha", "Beta", "CLAUDE.md", "Combined preview", "+ Add memory"])
        );
        const system = document.querySelector('[data-kind="system"]');
        expect(system?.querySelector(".memory-file-card-badge")?.textContent).toBe("system");
        const claude = document.querySelector('[data-kind="claude-config"]');
        expect(claude?.querySelector(".memory-file-card-badge")?.textContent).toContain("read-only");
        expect(claude?.querySelector(".fa-lock")).not.toBeNull();
        // "size · updated", from the entry itself.
        expect(document.querySelector('[data-id="g-a"] .memory-file-card-meta')?.textContent).toBe("5 B · 2h ago");
    });

    test("uses the Personal Memory tile grid and tile CSS, with no always-visible preview per entry", async () => {
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "# Big heading\n")]);
        render(() => <GlobalBundleManager />);
        await waitFor(() => expect(tileTitles()).toContain("Alpha"));

        expect(document.querySelector(".native-memory-manager-file-grid .memory-file-card")).not.toBeNull();
        // The 240px markdown preview per card is gone from the grid.
        expect(document.querySelector(".global-bundle-file-content")).toBeNull();
        expect(screen.queryByText("Big heading")).toBeNull();
    });

    test("clicking a tile expands it to the full view, with the Personal Memory back/breadcrumb header", async () => {
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "# Heading A\n")]);
        render(() => <GlobalBundleManager />);
        await openTile("Alpha");

        expect(await screen.findByText("← All global memory")).toBeInTheDocument();
        expect(document.querySelector(".native-memory-manager-crumbs")?.textContent).toContain("Alpha");
        expect(await screen.findByText("Heading A", { selector: ".heading" })).toBeInTheDocument();
        expect(historyMock).toHaveBeenCalledWith(undefined, { id: "g-a" });

        fireEvent.click(screen.getByText("← All global memory"));
        await waitFor(() => expect(tileTitles()).toContain("Alpha"));
    });

    test("keyboard activation opens a tile, same as a click", async () => {
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "a")]);
        render(() => <GlobalBundleManager />);
        await waitFor(() => expect(tileTitles()).toContain("Alpha"));
        fireEvent.keyDown(document.querySelector('[data-id="g-a"]')!, { key: "Enter" });
        expect(await screen.findByText("← All global memory")).toBeInTheDocument();
    });

    test("+ Add memory opens the full view as a new-entry editor, not an inline card", async () => {
        render(() => <GlobalBundleManager />);
        await openTile("+ Add memory");

        expect(await screen.findByText("← All global memory")).toBeInTheDocument();
        expect(screen.getByPlaceholderText("e.g. Coding Standards")).toBeInTheDocument();
        expect(textarea()).not.toBeNull();
        expect(screen.getByText("Add memory")).toBeInTheDocument();
    });

    test("saving a new entry creates it, appends it to the order, and opens it", async () => {
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "a")]);
        render(() => <GlobalBundleManager />);
        await openTile("+ Add memory");

        fireEvent.input(screen.getByPlaceholderText("e.g. Coding Standards"), { target: { value: "Gamma" } });
        fireEvent.input(textarea()!, { target: { value: "new body" } });
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "a"), bundle("new-id", "Gamma", "new body")]);
        fireEvent.click(screen.getByText("Add memory"));

        await waitFor(() => expect(reorderMock).toHaveBeenCalledWith(undefined, { ids: ["g-a", "new-id"] }));
        const sent = upsertMock.mock.calls[0][1] as Record<string, unknown>;
        expect(sent).toMatchObject({ id: "", name: "Gamma", instructions: "new body", is_global: true });
        expect(sent).not.toHaveProperty("base_sha256"); // nothing to be based on
        await waitFor(() => expect(document.querySelector(".native-memory-manager-crumbs")?.textContent).toContain("Gamma"));
    });

    test("drag-to-reorder moves an entry to the drop target's slot via reorderglobalbrain", async () => {
        listMemoriesMock.mockResolvedValue([
            bundle("g-a", "Alpha", "a", { sort_order: 0 }),
            bundle("g-b", "Beta", "b", { sort_order: 1 }),
            bundle("g-c", "Gamma", "c", { sort_order: 2 }),
        ]);
        render(() => <GlobalBundleManager />);
        await waitFor(() => expect(tileTitles()).toContain("Gamma"));

        const tile = (id: string) => document.querySelector<HTMLElement>(`[data-id="${id}"]`)!;
        expect(tile("g-c").getAttribute("draggable")).toBe("true");
        fireEvent.dragStart(tile("g-c"));
        fireEvent.dragOver(tile("g-a"));
        fireEvent.drop(tile("g-a"));

        await waitFor(() => expect(reorderMock).toHaveBeenCalledWith(undefined, { ids: ["g-c", "g-a", "g-b"] }));
    });

    test("system entries are not draggable (they never reorder)", async () => {
        listMemoriesMock.mockResolvedValue([bundle("sys-1", "Policy", "p", { is_system: true })]);
        render(() => <GlobalBundleManager />);
        await waitFor(() => expect(tileTitles()).toContain("Policy"));
        expect(document.querySelector('[data-id="sys-1"]')?.getAttribute("draggable")).toBeNull();
    });
});

describe("GlobalBundleManager — full view: pinned editor, base-carrying save, dirty drafts", () => {
    beforeEach(() => {
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "v1")]);
    });

    async function openAndEdit(): Promise<void> {
        render(() => <GlobalBundleManager />);
        await openTile("Alpha");
        fireEvent.click(await screen.findByText("Edit"));
        await waitFor(() => expect(textarea()).not.toBeNull());
    }

    test("the pinned layout's regions exist: actions + history on top, content at the bottom", async () => {
        render(() => <GlobalBundleManager />);
        await openTile("Alpha");

        const top = await screen.findByTestId("memory-pinned-top");
        const bottom = screen.getByTestId("memory-pinned-bottom");
        expect(top.textContent).toContain("Edit");
        expect(top.querySelector('[data-testid="memory-history"]')).not.toBeNull();
        expect(bottom.querySelector('[data-testid="memory-content"]')).not.toBeNull();
        expect(bottom.textContent).not.toContain("Edit");
        expect(document.querySelector('[role="separator"].memory-pinned-handle')).not.toBeNull();
    });

    test("save sends base_sha256 = the server's content_hash of what the draft started from", async () => {
        await openAndEdit();
        fireEvent.input(textarea()!, { target: { value: "v2" } });
        fireEvent.click(screen.getByText("Save"));

        await waitFor(() => expect(upsertMock).toHaveBeenCalled());
        expect(upsertMock.mock.calls[0][1]).toMatchObject({
            id: "g-a",
            name: "Alpha",
            instructions: "v2",
            is_global: true,
            base_sha256: serverHash("Alpha", "v1"),
        });
    });

    test("a system entry saves through upsertsystemmemory, with its base", async () => {
        listMemoriesMock.mockResolvedValue([bundle("sys-1", "Policy", "p1", { is_system: true })]);
        upsertSystemMock.mockImplementation((_c: unknown, b: Bundle) => Promise.resolve(b));
        render(() => <GlobalBundleManager />);
        await openTile("Policy");
        fireEvent.click(await screen.findByText("Edit"));
        fireEvent.input(textarea()!, { target: { value: "p2" } });
        fireEvent.click(screen.getByText("Save"));

        await waitFor(() => expect(upsertSystemMock).toHaveBeenCalled());
        expect(upsertMock).not.toHaveBeenCalled();
        expect(upsertSystemMock.mock.calls[0][1]).toMatchObject({ base_sha256: serverHash("Policy", "p1") });
    });

    test("Ctrl+S saves and Cmd+S saves", async () => {
        await openAndEdit();
        fireEvent.input(textarea()!, { target: { value: "v2" } });
        fireEvent.keyDown(textarea()!, { key: "s", ctrlKey: true });
        await waitFor(() => expect(upsertMock).toHaveBeenCalledTimes(1));

        fireEvent.click(await screen.findByText("Edit"));
        fireEvent.input(textarea()!, { target: { value: "v3" } });
        fireEvent.keyDown(textarea()!, { key: "s", metaKey: true });
        await waitFor(() => expect(upsertMock).toHaveBeenCalledTimes(2));
    });

    test("Esc cancels a clean draft at once, and asks first when the draft is dirty", async () => {
        await openAndEdit();
        const confirmSpy = vi.spyOn(window, "confirm");
        fireEvent.keyDown(textarea()!, { key: "Escape" });
        await waitFor(() => expect(textarea()).toBeNull());
        expect(confirmSpy).not.toHaveBeenCalled();

        fireEvent.click(await screen.findByText("Edit"));
        fireEvent.input(textarea()!, { target: { value: "dirty" } });
        confirmSpy.mockReturnValueOnce(false);
        fireEvent.keyDown(textarea()!, { key: "Escape" });
        expect(confirmSpy).toHaveBeenCalledTimes(1);
        expect(textarea()?.value).toBe("dirty");

        confirmSpy.mockReturnValueOnce(true);
        fireEvent.keyDown(textarea()!, { key: "Escape" });
        await waitFor(() => expect(textarea()).toBeNull());
    });

    test("a live change to the open entry keeps the dirty draft and shows the banner", async () => {
        await openAndEdit();
        fireEvent.input(textarea()!, { target: { value: "my draft" } });

        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "an agent wrote this")]);
        mpsHub.handlers.get("memories:changed")?.();

        expect(await screen.findByTestId("memory-conflict-banner")).toHaveTextContent(
            "This memory changed since you started editing."
        );
        expect(textarea()?.value).toBe("my draft");

        fireEvent.click(screen.getByText("View change"));
        const diff = screen.getByTestId("memory-conflict-diff").textContent ?? "";
        expect(diff).toContain("- v1");
        expect(diff).toContain("+ an agent wrote this");

        fireEvent.click(screen.getByText("Keep editing"));
        expect(screen.queryByTestId("memory-conflict-banner")).toBeNull();
        expect(textarea()?.value).toBe("my draft");
    });

    test("no banner when a change event leaves the entry's hash equal to the draft's base", async () => {
        await openAndEdit();
        fireEvent.input(textarea()!, { target: { value: "my draft" } });

        // Some OTHER entry changed; this one's name+instructions are identical.
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "v1", { updated_at: 99 }), bundle("g-z", "Zed", "z")]);
        const listCallsBefore = listMemoriesMock.mock.calls.length;
        mpsHub.handlers.get("memories:changed")?.();
        await waitFor(() => expect(listMemoriesMock.mock.calls.length).toBeGreaterThan(listCallsBefore));
        await new Promise((r) => setTimeout(r, 20));
        expect(screen.queryByTestId("memory-conflict-banner")).toBeNull();
        expect(textarea()?.value).toBe("my draft");
    });

    test("a save refused as a conflict keeps the draft and says it wasn't saved", async () => {
        await openAndEdit();
        fireEvent.input(textarea()!, { target: { value: "my draft" } });
        upsertMock.mockRejectedValueOnce(new Error("upsertmemory: conflict: Global Memory entry g-a changed since your edit began"));
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "someone else")]);
        fireEvent.click(screen.getByText("Save"));

        expect(await screen.findByTestId("memory-conflict-banner")).toHaveTextContent("Not saved");
        expect(textarea()?.value).toBe("my draft");

        // "Save anyway" rebases onto what's saved now and overwrites explicitly.
        await waitFor(() => expect(screen.getByText("Save anyway")).not.toBeDisabled());
        fireEvent.click(screen.getByText("Save anyway"));
        await waitFor(() => expect(upsertMock).toHaveBeenCalledTimes(2));
        expect(upsertMock.mock.calls[1][1]).toMatchObject({
            instructions: "my draft",
            base_sha256: serverHash("Alpha", "someone else"),
        });
    });
});

describe("GlobalBundleManager — overlapping refreshes", () => {
    test("a stale list that resolves last never overwrites a fresher one", async () => {
        listMemoriesMock.mockResolvedValueOnce([bundle("g-a", "Alpha", "v0")]);
        render(() => <GlobalBundleManager />);
        await waitFor(() => expect(tileTitles()).toContain("Alpha"));

        let resolveStale: (v: Bundle[]) => void = () => {};
        listMemoriesMock.mockImplementationOnce(() => new Promise<Bundle[]>((r) => (resolveStale = r)));
        listMemoriesMock.mockResolvedValueOnce([bundle("g-a", "Fresh", "v2")]);
        const onChanged = mpsHub.handlers.get("memories:changed");
        expect(onChanged).toBeDefined();
        onChanged!(); // the slow, older refresh
        onChanged!(); // the newer refresh, which answers first
        await waitFor(() => expect(tileTitles()).toContain("Fresh"));

        resolveStale([bundle("g-a", "Stale", "v1")]);
        await new Promise((r) => setTimeout(r, 0));
        expect(tileTitles()).toContain("Fresh");
        expect(tileTitles()).not.toContain("Stale");
    });
});

describe("GlobalBundleManager — read-only CLAUDE.md", () => {
    test("renders nothing for it before the fetch resolves", async () => {
        getClaudeGlobalConfigMock.mockReturnValue(new Promise(() => {}));
        render(() => <GlobalBundleManager />);
        await waitFor(() => expect(tileTitles()).toContain("+ Add memory"));
        expect(tileTitles()).not.toContain("CLAUDE.md");
    });

    test("its full view shows the path and the rendered content, read-only", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({ path: CLAUDE_CONFIG_PATH, content: "# Global rules\n", exists: true });
        render(() => <GlobalBundleManager />);
        await openTile("CLAUDE.md");

        expect(await screen.findByText(CLAUDE_CONFIG_PATH)).toBeInTheDocument();
        // Rendered as markdown: "# Global rules" becomes a heading.
        expect(screen.getByText("Global rules", { selector: ".heading" })).toBeInTheDocument();
        expect(document.querySelector("textarea")).toBeNull();
        expect(screen.queryByText("Edit")).toBeNull();
        expect(screen.queryByText("Save")).toBeNull();
    });

    test("its full view shows the empty state when no file exists at that path", async () => {
        render(() => <GlobalBundleManager />);
        await openTile("CLAUDE.md");
        expect((await screen.findByText("No file at this path yet.")).classList.contains("global-bundle-file-empty")).toBe(true);
    });

    test("the host CLI config path appears nowhere", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({ path: CLAUDE_CONFIG_PATH, content: "# Shared rules\n", exists: true });
        render(() => <GlobalBundleManager />);
        await openTile("CLAUDE.md");
        expect(await screen.findByText(CLAUDE_CONFIG_PATH)).toBeInTheDocument();
        expect(document.body.textContent).not.toContain("/home/user/.claude/CLAUDE.md");
    });
});

describe("GlobalBundleManager — combined preview tile", () => {
    test("opens the exact composed block", async () => {
        listMemoriesMock.mockResolvedValue([bundle("g-a", "Alpha", "alpha body")]);
        render(() => <GlobalBundleManager />);
        await openTile("Combined preview");
        expect(await screen.findByText("[Workspace] Alpha", { selector: ".heading" })).toBeInTheDocument();
    });
});
