// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Pinned and recent folders in the Files pane's Places (spec §5.5): the store, the sidebar and the menu items. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FilesPlaces } from "./FilesPlaces";
import type { FilesModel } from "./files-model";
import { buildFolderMenu, buildRowMenu } from "./files-menus";
import {
    addRecent,
    clearRecents,
    isPinned,
    MAX_RECENTS,
    pinnedPlaces,
    pinPlace,
    placeLabel,
    placeTitle,
    recentPlaces,
    resetPlacesForTests,
    SHOWN_RECENTS,
    togglePin,
    unpinPlace,
} from "./files-pins";

vi.mock("@/app/view/remotes/open-remotes", () => ({ openRemotesInPane: vi.fn() }));

beforeEach(() => resetPlacesForTests());
afterEach(() => cleanup());

describe("the pins store", () => {
    it("pins a folder once, in the order pinned, and keeps it in storage", () => {
        pinPlace({ path: "C:/Work" });
        pinPlace({ path: "c:/work/" }); // the same folder as Windows spells it
        pinPlace({ path: "/srv/app", connection: "user@box" });
        expect(pinnedPlaces().map((p) => p.path)).toEqual(["C:/Work", "/srv/app"]);
        expect(JSON.parse(localStorage.getItem("files:places:pinned") ?? "[]")).toHaveLength(2);
    });

    it("keeps a folder on a host apart from the same path on this computer", () => {
        pinPlace({ path: "/srv/app" });
        expect(isPinned({ path: "/srv/app", connection: "user@box" })).toBe(false);
        pinPlace({ path: "/srv/app", connection: "user@box" });
        expect(pinnedPlaces()).toHaveLength(2);
    });

    it("unpins and toggles", () => {
        pinPlace({ path: "/a" });
        togglePin({ path: "/a" });
        expect(isPinned({ path: "/a" })).toBe(false);
        togglePin({ path: "/a" });
        expect(isPinned({ path: "/a" })).toBe(true);
        unpinPlace({ path: "/a" });
        expect(pinnedPlaces()).toEqual([]);
    });

    it("lists recents newest first, once each, and no more than it keeps", () => {
        for (let i = 0; i < MAX_RECENTS + 3; i++) addRecent({ path: `/r/${i}` });
        addRecent({ path: "/r/5" }); // opened again: moves to the front
        const shown = recentPlaces().map((p) => p.path);
        expect(shown[0]).toBe("/r/5");
        expect(new Set(shown).size).toBe(shown.length);
        expect(shown).toHaveLength(SHOWN_RECENTS);
        expect(JSON.parse(localStorage.getItem("files:places:recent") ?? "[]")).toHaveLength(MAX_RECENTS);
    });

    it("leaves a pinned folder out of the recents, and clears them", () => {
        addRecent({ path: "/a" });
        addRecent({ path: "/b" });
        pinPlace({ path: "/b" });
        expect(recentPlaces().map((p) => p.path)).toEqual(["/a"]);
        clearRecents();
        expect(recentPlaces()).toEqual([]);
    });

    it("labels by the folder's name and says the host in the tooltip", () => {
        expect(placeLabel({ path: "C:/Users/a/Projects" })).toBe("Projects");
        expect(placeLabel({ path: "/" })).toBe("/");
        expect(placeTitle({ path: "/srv/app", connection: "user@box" })).toBe("/srv/app on user@box");
        expect(placeTitle({ path: "/srv/app" })).toBe("/srv/app");
    });
});

function fakeModel(over: Partial<Record<string, unknown>> = {}) {
    const openOn = vi.fn(async () => {});
    const model = {
        blockId: "b1",
        showSidebar: () => true,
        places: () => [{ id: "home", label: "Home", path: "/home/a", kind: "home" }],
        remotes: () => [],
        agents: () => [],
        connection: () => "",
        openOn,
        ...over,
    } as unknown as FilesModel;
    return { model, openOn };
}

describe("Places", () => {
    it("shows nothing extra until something is pinned or opened", () => {
        render(() => <FilesPlaces model={fakeModel().model} />);
        expect(screen.queryByText("Pinned")).toBeNull();
        expect(screen.queryByText("Recent")).toBeNull();
    });

    it("opens a pinned folder on its own host, and unpins it from its row", () => {
        pinPlace({ path: "/srv/app", connection: "user@box" });
        const { model, openOn } = fakeModel();
        render(() => <FilesPlaces model={model} />);
        expect(screen.getByText("Pinned")).toBeTruthy();
        fireEvent.click(screen.getByText("app"));
        expect(openOn).toHaveBeenCalledWith("user@box", "/srv/app");
        fireEvent.click(screen.getByRole("button", { name: "Unpin app" }));
        expect(pinnedPlaces()).toEqual([]);
        expect(screen.queryByText("Pinned")).toBeNull();
    });

    it("opens a recent folder on this computer", () => {
        addRecent({ path: "/home/a/notes" });
        const { model, openOn } = fakeModel();
        render(() => <FilesPlaces model={model} />);
        expect(screen.getByText("Recent")).toBeTruthy();
        fireEvent.click(screen.getByText("notes"));
        expect(openOn).toHaveBeenCalledWith("", "/home/a/notes");
        expect(screen.queryByRole("button", { name: /Unpin/ })).toBeNull();
    });
});

describe("the menus", () => {
    const entry = { name: "src", is_dir: true, is_symlink: false, hidden: false, readonly: false };
    const menuModel = () =>
        ({
            blockId: "b1",
            path: () => "/home/a",
            pathOf: (n: string) => `/home/a/${n}`,
            connection: () => "",
            root: () => undefined,
            selectedEntries: () => [],
            showHidden: () => false,
        }) as unknown as FilesModel;
    const actions = { openEntry: vi.fn(), openInNewTab: vi.fn(), copyPaths: vi.fn(), askDeletePermanently: vi.fn() };
    const label = (items: { label?: string }[], re: RegExp) => items.find((i) => i.label && re.test(i.label)) as { label: string; onSelect: () => void } | undefined;

    it("pins a folder row from its menu, and offers to unpin it after", () => {
        const pin = label(buildRowMenu(menuModel(), entry, actions), /Pin to Places/)!;
        pin.onSelect();
        expect(isPinned({ path: "/home/a/src" })).toBe(true);
        expect(label(buildRowMenu(menuModel(), entry, actions), /Unpin from Places/)).toBeTruthy();
    });

    it("has no pin item for a file", () => {
        const file = { ...entry, name: "a.txt", is_dir: false };
        expect(label(buildRowMenu(menuModel(), file, actions), /Pin|Unpin/)).toBeUndefined();
    });

    it("pins the folder shown from the empty-space menu", () => {
        label(buildFolderMenu(menuModel(), actions), /Pin to Places/)!.onSelect();
        expect(isPinned({ path: "/home/a" })).toBe(true);
    });
});
