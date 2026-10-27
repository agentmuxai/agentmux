// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The files a task carries (SPEC_KANBAN_TASK_MEDIA_2026_10_25.md §4): up to three thumbnails and "+N" on
 * the card, a quiet pill on a UI task without any, and the drawer's list, newest attempt first.
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openMediaInEditor = vi.fn();
vi.mock("@/app/element/local-media", () => ({
    fetchMediaBlob: () => Promise.reject(new Error("no srv in a test")),
    INLINE_IMAGE_MAX_BYTES: 25 * 1024 * 1024,
}));
vi.mock("@/app/element/markdown-media", () => ({
    MarkdownImg: (p: { props: { src: string; alt: string } }) => {
        const el = document.createElement("span");
        el.setAttribute("data-testid", "media");
        el.textContent = p.props.src;
        return el;
    },
    openMediaInEditor: (path: string) => openMediaInEditor(path),
}));

import { CardMedia, DrawerMedia, mediaNewestFirst, mediaOverflow, noMediaWords } from "./kanban-media";
import type { KanbanMediaItem, KanbanMediaPreview, KanbanTask } from "./kanban-types";

afterEach(() => {
    cleanup();
    openMediaInEditor.mockClear();
});

const preview = (n: number, kind: KanbanMediaPreview["kind"] = "image"): KanbanMediaPreview => ({
    sha256: `sha${n}`,
    kind,
    mime: kind === "image" ? "image/png" : "video/mp4",
    path: `C:/shared/task-media/sha${n}`,
    thumb: "",
});

function task(media: Partial<NonNullable<KanbanTask["card"]["media"]>> | null, state: KanbanTask["state"] = "review"): KanbanTask {
    return {
        id: "w1",
        title: "Fix the narrow list",
        state,
        card: {
            num: 7,
            media: media ? { count: 0, first: [], ui_change: false, no_media_reason: "", ...media } : undefined,
        },
    } as unknown as KanbanTask;
}

const item = (id: number, attempt: number, over: Partial<KanbanMediaItem> = {}): KanbanMediaItem => ({
    id,
    work_id: "w1",
    sha256: `sha${id}`,
    name: `shot${id}.png`,
    kind: "image",
    mime: "image/png",
    size: 10,
    caption: `caption ${id}`,
    attached_by: "Naki-2",
    attempt,
    at: 1000 + id,
    removed_at: null,
    path: `C:/shared/task-media/sha${id}.png`,
    thumb: "",
    ...over,
});

describe("the card's strip", () => {
    it("shows up to three thumbnails and the rest as +N", () => {
        const t = task({ count: 5, first: [preview(1), preview(2, "recording"), preview(3)] });
        const { container } = render(() => <CardMedia task={t} />);
        expect(container.querySelectorAll(".kanban-media__thumb")).toHaveLength(3);
        expect(container.querySelectorAll(".kanban-media__thumb--recording")).toHaveLength(1);
        expect(screen.getByText("+2")).toBeTruthy();
    });

    it("shows nothing for a task with no media, so the board does not change for it", () => {
        const { container } = render(() => <CardMedia task={task({ count: 0 }, "claimed")} />);
        expect(container.textContent).toBe("");
        expect(mediaOverflow(3, 3)).toBe(0);
    });

    it("clicking a thumbnail opens the file in the Editor and does not open the card", () => {
        const onCard = vi.fn();
        const t = task({ count: 1, first: [preview(1)] });
        const { container } = render(() => (
            <div onClick={onCard}>
                <CardMedia task={t} />
            </div>
        ));
        fireEvent.click(container.querySelector(".kanban-media__thumb")!);
        expect(openMediaInEditor).toHaveBeenCalledWith("C:/shared/task-media/sha1");
        expect(onCard).not.toHaveBeenCalled();
    });

    it("says 'no media' on a UI task that has begun and has none, with the reason when there is one", () => {
        expect(noMediaWords(task({ ui_change: true }, "review"))).toBe("no media");
        expect(noMediaWords(task({ ui_change: true, no_media_reason: "headless host" }, "review"))).toBe("No media: headless host");
        expect(noMediaWords(task({ ui_change: true }, "open"))).toBeNull();
        expect(noMediaWords(task({ ui_change: false }, "review"))).toBeNull();
        expect(noMediaWords(task({ ui_change: true, count: 1 }, "review"))).toBeNull();
        // Sent back and handed in again without new pictures: the reason shows beside the old ones.
        expect(noMediaWords(task({ ui_change: true, count: 2, no_media_reason: "not captured this time" }, "review"))).toBe(
            "No media: not captured this time"
        );
        expect(noMediaWords(task(null, "review"))).toBeNull();
        const { container } = render(() => <CardMedia task={task({ ui_change: true }, "review")} />);
        expect(container.textContent).toContain("no media");
    });
});

describe("the drawer's Media section", () => {
    const mount = (t: KanbanTask, items: KanbanMediaItem[]) => {
        const onAct = vi.fn(() => Promise.resolve({ ok: true }));
        render(() => <DrawerMedia task={t} items={items} now={100_000} busy={false} onAct={onAct} />);
        return onAct;
    };

    it("lists the newest attempt first, with who attached it, when and on which attempt", () => {
        const items = [item(1, 1), item(2, 2), item(3, 1)];
        expect(mediaNewestFirst(items).map((i) => i.id)).toEqual([2, 3, 1]);
        mount(task({ count: 3, ui_change: true }), items);
        const shown = screen.getAllByTestId("media").map((e) => e.textContent);
        expect(shown).toEqual(["C:/shared/task-media/sha2.png", "C:/shared/task-media/sha3.png", "C:/shared/task-media/sha1.png"]);
        expect(screen.getByText("caption 2")).toBeTruthy();
        expect(screen.getAllByText(/Naki-2, attempt/).length).toBe(3);
    });

    it("opens a file in the Editor on Open, and removes it on Remove", () => {
        const onAct = mount(task({ count: 1, ui_change: true }), [item(9, 1)]);
        fireEvent.click(screen.getByText("Open"));
        expect(openMediaInEditor).toHaveBeenCalledWith("C:/shared/task-media/sha9.png");
        fireEvent.click(screen.getByText("Remove"));
        expect(onAct).toHaveBeenCalledWith({ action: "media_remove", media_id: 9 });
    });

    it("shows a removed file as removed, with no picture and no buttons", () => {
        mount(task({ count: 0, ui_change: true }), [item(4, 1, { removed_at: 5000, removed_by: "operator", path: "" })]);
        // One box: the name, who removed it and when, and who attached it.
        expect(screen.getByText(/removed by operator, \d+[a-z ]* ago/)).toBeTruthy();
        expect(screen.getByText("shot4.png")).toBeTruthy();
        expect(screen.getByText(/attached by Naki-2, attempt 1/)).toBeTruthy();
        expect(screen.queryByTestId("media")).toBeNull();
        expect(screen.queryByText("Remove")).toBeNull();
    });

    it("lets the operator mark or unmark the task as a UI change", () => {
        const onAct = mount(task({ count: 0, ui_change: false }, "claimed"), []);
        fireEvent.click(screen.getByRole("checkbox"));
        expect(onAct).toHaveBeenCalledWith({ action: "ui_change", ui_change: true });
    });

    it("is not there for a task that has not begun and has no media", () => {
        const { container } = render(() => (
            <DrawerMedia task={task({ count: 0 }, "open")} items={[]} now={1} busy={false} onAct={() => Promise.resolve(null)} />
        ));
        expect(container.textContent).toBe("");
    });
});
