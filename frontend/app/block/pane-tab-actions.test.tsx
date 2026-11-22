// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import * as util from "@/util/util";
import { KeptEndIcons, PaneTabActions } from "./pane-tab-actions";

afterEach(() => cleanup());

const nodeModel = (closing = false) => ({ isClosing: () => closing }) as never;
const vm = (buttons: unknown[] | undefined) => ({ endIconButtons: buttons ? util.createSignalAtom(buttons) : undefined }) as never;
const btn = (title: string, className?: string, click = () => {}) => ({ elemtype: "iconbutton", icon: "search", title, click, className });

describe("PaneTabActions", () => {
    it("draws each of the view's buttons, as a named group", () => {
        const { container } = render(() => <PaneTabActions viewModel={vm([btn("Find"), btn("Messages")])} nodeModel={nodeModel()} />);
        const group = container.querySelector(".pane-tab-actions")!;
        expect(group.getAttribute("role")).toBe("group");
        expect(group.getAttribute("aria-label")).toBe("Tab actions");
        expect(group.querySelectorAll(".pane-tab-action")).toHaveLength(2);
    });

    it("draws nothing for a view with no buttons, or while the pane closes", () => {
        const a = render(() => <PaneTabActions viewModel={vm(undefined)} nodeModel={nodeModel()} />);
        expect(a.container.querySelector(".pane-tab-actions")).toBeNull();
        const b = render(() => <PaneTabActions viewModel={vm([btn("Find")])} nodeModel={nodeModel(true)} />);
        expect(b.container.querySelector(".pane-tab-actions")).toBeNull();
        const c = render(() => <PaneTabActions viewModel={null} nodeModel={nodeModel()} />);
        expect(c.container.querySelector(".pane-tab-actions")).toBeNull();
    });

    it("runs a button's own click, and keeps the press from the tab it sits in", () => {
        const click = vi.fn();
        const onTab = vi.fn();
        const { container } = render(() => (
            <div onMouseDown={onTab} onClick={onTab} onDblClick={onTab} onContextMenu={onTab}>
                <PaneTabActions viewModel={vm([btn("Find", undefined, click)])} nodeModel={nodeModel()} />
            </div>
        ));
        const b = container.querySelector(".pane-tab-action") as HTMLElement;
        fireEvent.mouseDown(b);
        fireEvent.click(b);
        fireEvent.dblClick(b);
        fireEvent.contextMenu(b);
        expect(click).toHaveBeenCalledTimes(1);
        expect(onTab).not.toHaveBeenCalled();
    });
});

describe("KeptEndIcons", () => {
    it("draws only the buttons flagged as the only way in to something", () => {
        const { container } = render(() => (
            <KeptEndIcons viewModel={vm([btn("Find"), btn("Messages", "messages-btn block-frame-end-keep")])} nodeModel={nodeModel()} />
        ));
        expect(container.querySelectorAll(".block-frame-end-kept button")).toHaveLength(1);
        expect(container.querySelector(".messages-btn")).not.toBeNull();
    });

    it("draws nothing when no button is flagged", () => {
        const { container } = render(() => <KeptEndIcons viewModel={vm([btn("Find")])} nodeModel={nodeModel()} />);
        expect(container.querySelector(".block-frame-end-kept")).toBeNull();
        expect(screen.queryByTitle("Find")).toBeNull();
    });
});
